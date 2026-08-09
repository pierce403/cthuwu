import { randomUUID } from "node:crypto";
import { createInterface, type Interface } from "node:readline";
import type { Readable, Writable } from "node:stream";

export type InboundText = {
  type: "inbound_text" | "reject_inbound" | "reject_oversized";
  id: string;
  messageId: string;
  senderInboxId: string;
  senderAddress?: string;
  sentAtNs: string;
  deadlineUnixMs: number;
  conversationId: string;
  text: string;
};

export type SidecarResponse =
  | { type: "reply"; id: string; text: string }
  | { type: "ignore"; id: string };

export type BridgeOptions = {
  input: Readable;
  output: Writable;
  timeoutMs?: number;
  diagnostic?: (message: string) => void;
  idFactory?: () => string;
  maxReplyBytes?: number;
  maxLineBytes?: number;
  maxPending?: number;
  now?: () => number;
};

type Pending = {
  resolve: (response: SidecarResponse) => void;
  reject: (error: Error) => void;
  timer: NodeJS.Timeout;
};

const DEFAULT_TIMEOUT_MS = 300_000;
const DEFAULT_MAX_REPLY_BYTES = 16 * 1024;
const DEFAULT_MAX_LINE_BYTES = 256 * 1024;
const DEFAULT_MAX_PENDING = 2;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseTimeout(value: string | undefined): number {
  if (value === undefined) {
    return DEFAULT_TIMEOUT_MS;
  }
  if (!/^\d+$/u.test(value)) {
    throw new Error("UWUBOT_REPLY_TIMEOUT_MS must be an integer number of milliseconds");
  }
  const timeout = Number(value);
  if (!Number.isSafeInteger(timeout) || timeout < 2_000 || timeout > 300_000) {
    throw new Error("UWUBOT_REPLY_TIMEOUT_MS must be between 2000 and 300000");
  }
  return timeout;
}

export class JsonlBridge {
  readonly #output: Writable;
  readonly #timeoutMs: number;
  readonly #diagnostic: (message: string) => void;
  readonly #idFactory: () => string;
  readonly #maxReplyBytes: number;
  readonly #maxLineBytes: number;
  readonly #maxPending: number;
  readonly #now: () => number;
  readonly #reader: Interface;
  readonly #pending = new Map<string, Pending>();
  #closed = false;
  #rejecting = false;

  constructor(options: BridgeOptions) {
    this.#output = options.output;
    this.#timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.#diagnostic = options.diagnostic ?? (() => undefined);
    this.#idFactory = options.idFactory ?? randomUUID;
    this.#maxReplyBytes = options.maxReplyBytes ?? DEFAULT_MAX_REPLY_BYTES;
    this.#maxLineBytes = options.maxLineBytes ?? DEFAULT_MAX_LINE_BYTES;
    this.#maxPending = options.maxPending ?? DEFAULT_MAX_PENDING;
    this.#now = options.now ?? Date.now;
    if (!Number.isInteger(this.#maxPending) || this.#maxPending < 1) {
      throw new Error("maxPending must be a positive integer");
    }
    this.#reader = createInterface({ input: options.input, crlfDelay: Infinity });
    this.#reader.on("line", (line) => this.#handleLine(line));
    this.#reader.on("close", () => {
      this.close(new Error("uwubot closed the sidecar protocol input"));
    });
  }

  async request(
    message: Omit<InboundText, "type" | "id" | "deadlineUnixMs">,
  ): Promise<SidecarResponse> {
    return this.#request("inbound_text", message);
  }

  async rejectOversized(
    message: Omit<InboundText, "type" | "id" | "deadlineUnixMs" | "text">,
  ): Promise<SidecarResponse> {
    return this.#request("reject_oversized", { ...message, text: "" });
  }

  async #request(
    eventType: "inbound_text" | "reject_oversized",
    message: Omit<InboundText, "type" | "id" | "deadlineUnixMs">,
  ): Promise<SidecarResponse> {
    if (this.#closed) {
      throw new Error("sidecar protocol is closed");
    }
    if (this.#pending.size >= this.#maxPending) {
      if (this.#rejecting) {
        throw new Error("sidecar rejection capacity is already occupied");
      }
      this.#rejecting = true;
      try {
        const id = this.#idFactory();
        const acknowledgement = this.#pendingResponse(id);
        try {
          await this.#writeLine({
            type: "reject_inbound",
            id,
            ...message,
            text: "",
            deadlineUnixMs: this.#now() + this.#timeoutMs,
          });
        } catch (error) {
          this.#rejectPending(id, error);
        }
        return await acknowledgement;
      } finally {
        this.#rejecting = false;
      }
    }
    const id = this.#idFactory();
    if (this.#pending.has(id)) {
      throw new Error("sidecar protocol generated a duplicate request ID");
    }

    const response = this.#pendingResponse(id);

    try {
      await this.#writeLine({
        type: eventType,
        id,
        ...message,
        deadlineUnixMs: this.#now() + this.#timeoutMs,
      });
    } catch (error) {
      this.#rejectPending(id, error);
    }
    return response;
  }

  #pendingResponse(id: string): Promise<SidecarResponse> {
    if (this.#pending.has(id)) {
      throw new Error("sidecar protocol generated a duplicate request ID");
    }
    return new Promise<SidecarResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`uwubot did not answer request ${id} before the timeout`));
      }, this.#timeoutMs);
      this.#pending.set(id, { resolve, reject, timer });
    });
  }

  #rejectPending(id: string, reason: unknown): void {
    const pending = this.#pending.get(id);
    if (pending === undefined) {
      return;
    }
    this.#pending.delete(id);
    clearTimeout(pending.timer);
    pending.reject(
      reason instanceof Error ? reason : new Error("failed to write sidecar request"),
    );
  }

  close(reason = new Error("sidecar protocol closed")): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#reader.close();
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(reason);
    }
    this.#pending.clear();
  }

  #handleLine(line: string): void {
    if (Buffer.byteLength(line, "utf8") > this.#maxLineBytes) {
      this.#diagnostic("ignored an oversized line from uwubot");
      return;
    }

    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch {
      this.#diagnostic("ignored malformed JSON from uwubot");
      return;
    }
    if (!isRecord(value) || typeof value.id !== "string" || value.id.length > 128) {
      this.#diagnostic("ignored an invalid response from uwubot");
      return;
    }

    let response: SidecarResponse;
    if (value.type === "ignore") {
      response = { type: "ignore", id: value.id };
    } else if (value.type === "reply" && typeof value.text === "string") {
      if (Buffer.byteLength(value.text, "utf8") > this.#maxReplyBytes) {
        this.#rejectResponse(
          value.id,
          new Error("uwubot reply exceeds the 16 KiB XMTP response limit"),
          "rejected an oversized reply from uwubot",
        );
        return;
      }
      response = { type: "reply", id: value.id, text: value.text };
    } else {
      this.#rejectResponse(
        value.id,
        new Error("uwubot returned an invalid response"),
        "rejected an invalid response from uwubot",
      );
      return;
    }

    const pending = this.#pending.get(response.id);
    if (pending === undefined) {
      this.#diagnostic("ignored a response for an unknown or expired request");
      return;
    }
    this.#pending.delete(response.id);
    clearTimeout(pending.timer);
    pending.resolve(response);
  }

  #rejectResponse(id: string, error: Error, diagnostic: string): void {
    const pending = this.#pending.get(id);
    if (pending === undefined) {
      this.#diagnostic("ignored an invalid response for an unknown or expired request");
      return;
    }
    this.#pending.delete(id);
    clearTimeout(pending.timer);
    pending.reject(error);
    this.#diagnostic(diagnostic);
  }

  async #writeLine(value: InboundText): Promise<void> {
    const line = `${JSON.stringify(value)}\n`;
    if (Buffer.byteLength(line, "utf8") > this.#maxLineBytes) {
      throw new Error("sidecar request exceeds the bounded JSONL frame size");
    }
    await new Promise<void>((resolve, reject) => {
      this.#output.write(line, "utf8", (error) => {
        if (error === null || error === undefined) {
          resolve();
        } else {
          reject(error);
        }
      });
    });
  }
}
