import { randomUUID } from "node:crypto";
import { createInterface, type Interface } from "node:readline";
import type { Readable, Writable } from "node:stream";

export type InboundText = {
  type: "inbound_text";
  id: string;
  messageId: string;
  senderInboxId: string;
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
};

type Pending = {
  resolve: (response: SidecarResponse) => void;
  reject: (error: Error) => void;
  timer: NodeJS.Timeout;
};

const DEFAULT_TIMEOUT_MS = 90_000;
const DEFAULT_MAX_REPLY_BYTES = 16 * 1024;
const DEFAULT_MAX_LINE_BYTES = 256 * 1024;
const DEFAULT_MAX_PENDING = 2;

export class BridgeBusyError extends Error {}

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
  if (!Number.isSafeInteger(timeout) || timeout < 1_000 || timeout > 300_000) {
    throw new Error("UWUBOT_REPLY_TIMEOUT_MS must be between 1000 and 300000");
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
  readonly #reader: Interface;
  readonly #pending = new Map<string, Pending>();
  #closed = false;

  constructor(options: BridgeOptions) {
    this.#output = options.output;
    this.#timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.#diagnostic = options.diagnostic ?? (() => undefined);
    this.#idFactory = options.idFactory ?? randomUUID;
    this.#maxReplyBytes = options.maxReplyBytes ?? DEFAULT_MAX_REPLY_BYTES;
    this.#maxLineBytes = options.maxLineBytes ?? DEFAULT_MAX_LINE_BYTES;
    this.#maxPending = options.maxPending ?? DEFAULT_MAX_PENDING;
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
    message: Omit<InboundText, "type" | "id">,
  ): Promise<SidecarResponse> {
    if (this.#closed) {
      throw new Error("sidecar protocol is closed");
    }
    if (this.#pending.size >= this.#maxPending) {
      throw new BridgeBusyError("uwubot is already processing the maximum pending work");
    }
    const id = this.#idFactory();
    if (this.#pending.has(id)) {
      throw new Error("sidecar protocol generated a duplicate request ID");
    }

    let pending!: Pending;
    const response = new Promise<SidecarResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`uwubot did not answer request ${id} before the timeout`));
      }, this.#timeoutMs);
      pending = { resolve, reject, timer };
      this.#pending.set(id, pending);
    });

    try {
      await this.#writeLine({ type: "inbound_text", id, ...message });
    } catch (error) {
      if (this.#pending.delete(id)) {
        clearTimeout(pending.timer);
        pending.reject(
          error instanceof Error ? error : new Error("failed to write sidecar request"),
        );
      }
    }
    return response;
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
