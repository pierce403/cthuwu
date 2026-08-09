import { PassThrough } from "node:stream";
import { describe, expect, it } from "vitest";
import { JsonlBridge, parseTimeout, type InboundText } from "./protocol.js";

function nextLine(stream: PassThrough): Promise<string> {
  return new Promise((resolve) => {
    let buffer = "";
    const onData = (chunk: Buffer): void => {
      buffer += chunk.toString("utf8");
      const newline = buffer.indexOf("\n");
      if (newline !== -1) {
        stream.off("data", onData);
        resolve(buffer.slice(0, newline));
      }
    };
    stream.on("data", onData);
  });
}

function message(): Omit<InboundText, "type" | "id" | "deadlineUnixMs"> {
  return {
    messageId: "message-1",
    senderInboxId: "inbox-1",
    senderAddress: "0x4200000000000000000000000000000000000006",
    sentAtNs: "1750000000000000000",
    conversationId: "conversation-1",
    text: "hello from the human realm",
  };
}

describe("JSONL uwubot bridge", () => {
  it("emits one inbound request and resolves a matching reply", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const line = nextLine(output);
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-1",
      timeoutMs: 1_000,
      now: () => 10_000,
    });

    const response = bridge.request(message());
    expect(JSON.parse(await line)).toEqual({
      type: "inbound_text",
      id: "request-1",
      ...message(),
      deadlineUnixMs: 11_000,
    });
    input.write('{"type":"reply","id":"request-1","text":"a tiny tentacle waves"}\n');

    await expect(response).resolves.toEqual({
      type: "reply",
      id: "request-1",
      text: "a tiny tentacle waves",
    });
    bridge.close();
  });

  it("resolves ignore without synthesizing a reply", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-2",
      timeoutMs: 1_000,
    });
    const response = bridge.request(message());
    input.write('{"type":"ignore","id":"request-2"}\n');

    await expect(response).resolves.toEqual({ type: "ignore", id: "request-2" });
    bridge.close();
  });

  it("sends an oversized-message tombstone without forwarding its content", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const line = nextLine(output);
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-oversized",
      timeoutMs: 1_000,
      now: () => 10_000,
    });
    const response = bridge.rejectOversized({
      messageId: "message-oversized",
      senderInboxId: "inbox-1",
      senderAddress: "0x4200000000000000000000000000000000000006",
      sentAtNs: "1750000000000000000",
      conversationId: "conversation-1",
    });
    expect(JSON.parse(await line)).toEqual({
      type: "reject_oversized",
      id: "request-oversized",
      messageId: "message-oversized",
      senderInboxId: "inbox-1",
      senderAddress: "0x4200000000000000000000000000000000000006",
      sentAtNs: "1750000000000000000",
      conversationId: "conversation-1",
      text: "",
      deadlineUnixMs: 11_000,
    });
    input.write('{"type":"ignore","id":"request-oversized"}\n');
    await expect(response).resolves.toEqual({
      type: "ignore",
      id: "request-oversized",
    });
    bridge.close();
  });

  it("rejects oversized responses immediately without echoing their content", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const diagnostics: string[] = [];
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-3",
      timeoutMs: 1_000,
      maxReplyBytes: 8,
      diagnostic: (value) => diagnostics.push(value),
    });
    const response = bridge.request(message());
    input.write("secret malformed body\n");
    input.write(`${JSON.stringify({
      type: "reply",
      id: "request-3",
      text: "far too long",
    })}\n`);

    await expect(response).rejects.toThrow("exceeds the 16 KiB");
    expect(diagnostics).toEqual([
      "ignored malformed JSON from uwubot",
      "rejected an oversized reply from uwubot",
    ]);
    expect(diagnostics.join(" ")).not.toContain("secret");
    bridge.close();
  });

  it("rejects pending work when uwubot closes the protocol", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-4",
      timeoutMs: 1_000,
    });
    const response = bridge.request(message());
    input.end();
    await expect(response).rejects.toThrow("closed the sidecar protocol input");
  });

  it("times out a request when uwubot never responds", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-5",
      timeoutMs: 10,
    });
    const response = bridge.request(message());
    await expect(response).rejects.toThrow("before the timeout");
    bridge.close();
  });

  it("bounds pending work instead of building an unbounded queue", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const rejection = new Promise<InboundText>((resolve) => {
      let buffer = "";
      output.on("data", (chunk: Buffer) => {
        buffer += chunk.toString("utf8");
        for (const line of buffer.split("\n").filter(Boolean)) {
          const frame = JSON.parse(line) as InboundText;
          if (frame.type === "reject_inbound") {
            resolve(frame);
          }
        }
      });
    });
    let sequence = 0;
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => `request-busy-${sequence++}`,
      timeoutMs: 1_000,
      maxPending: 1,
    });
    const first = bridge.request(message());
    const second = bridge.request({ ...message(), messageId: "message-2" });
    const rejectionFrame = await rejection;
    expect(rejectionFrame.text).toBe("");
    input.write(
      `${JSON.stringify({
        type: "reply",
        id: rejectionFrame.id,
        text: "CTHUWU IS BUSY. RETRY WITH A NEW MESSAGE.",
      })}\n`,
    );
    await expect(second).resolves.toEqual({
      type: "reply",
      id: rejectionFrame.id,
      text: "CTHUWU IS BUSY. RETRY WITH A NEW MESSAGE.",
    });
    bridge.close();
    await expect(first).rejects.toThrow("protocol closed");
  });

  it("passes through a duplicate rejection ignore without a second busy reply", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    const rejection = new Promise<InboundText>((resolve) => {
      let buffer = "";
      output.on("data", (chunk: Buffer) => {
        buffer += chunk.toString("utf8");
        for (const line of buffer.split("\n").filter(Boolean)) {
          const frame = JSON.parse(line) as InboundText;
          if (frame.type === "reject_inbound") {
            resolve(frame);
          }
        }
      });
    });
    let sequence = 0;
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => `request-duplicate-${sequence++}`,
      timeoutMs: 1_000,
      maxPending: 1,
    });
    const first = bridge.request(message());
    const duplicate = bridge.request(message());
    const rejectionFrame = await rejection;
    input.write(`${JSON.stringify({ type: "ignore", id: rejectionFrame.id })}\n`);
    await expect(duplicate).resolves.toEqual({
      type: "ignore",
      id: rejectionFrame.id,
    });
    input.write(
      `${JSON.stringify({
        type: "reply",
        id: "request-duplicate-0",
        text: "first delivery completed",
      })}\n`,
    );
    await expect(first).resolves.toMatchObject({
      type: "reply",
      text: "first delivery completed",
    });
    bridge.close();
  });

  it("rejects an oversized outbound frame without poisoning the next request", async () => {
    const input = new PassThrough();
    const output = new PassThrough();
    let sequence = 0;
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => `request-frame-${sequence++}`,
      timeoutMs: 1_000,
      maxLineBytes: 512,
    });
    await expect(
      bridge.request({ ...message(), text: "x".repeat(1_000) }),
    ).rejects.toThrow("bounded JSONL frame");

    const line = nextLine(output);
    const response = bridge.request(message());
    const frame = JSON.parse(await line) as InboundText;
    input.write(`${JSON.stringify({ type: "ignore", id: frame.id })}\n`);
    await expect(response).resolves.toEqual({ type: "ignore", id: frame.id });
    bridge.close();
  });
});

describe("timeout configuration", () => {
  it("uses a bounded operator-provided timeout", () => {
    expect(parseTimeout(undefined)).toBe(300_000);
    expect(parseTimeout("2000")).toBe(2_000);
    expect(parseTimeout("300000")).toBe(300_000);
    expect(() => parseTimeout("1999")).toThrow("between");
    expect(() => parseTimeout("wat")).toThrow("integer");
  });
});
