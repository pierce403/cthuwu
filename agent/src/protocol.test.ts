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

function message(): Omit<InboundText, "type" | "id"> {
  return {
    messageId: "message-1",
    senderInboxId: "inbox-1",
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
    });

    const response = bridge.request(message());
    expect(JSON.parse(await line)).toEqual({
      type: "inbound_text",
      id: "request-1",
      ...message(),
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
    const bridge = new JsonlBridge({
      input,
      output,
      idFactory: () => "request-busy",
      timeoutMs: 1_000,
      maxPending: 1,
    });
    const first = bridge.request(message());
    await expect(bridge.request(message())).rejects.toThrow("maximum pending work");
    bridge.close();
    await expect(first).rejects.toThrow("protocol closed");
  });
});

describe("timeout configuration", () => {
  it("uses a bounded operator-provided timeout", () => {
    expect(parseTimeout(undefined)).toBe(90_000);
    expect(parseTimeout("1000")).toBe(1_000);
    expect(parseTimeout("300000")).toBe(300_000);
    expect(() => parseTimeout("999")).toThrow("between");
    expect(() => parseTimeout("wat")).toThrow("integer");
  });
});
