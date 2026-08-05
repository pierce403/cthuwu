import type { LogLevel } from "@xmtp/agent-sdk";
import { prepareAgentEnvironment } from "./identity.js";
import { JsonlBridge, parseTimeout } from "./protocol.js";

const MAX_INBOUND_TEXT_BYTES = 16 * 1024;

function diagnostic(message: string): void {
  process.stderr.write(`[cthuwu-xmtp] ${message.replace(/[\r\n]+/gu, " ")}\n`);
}

function isolateProtocolOutput(): void {
  const sdkDiagnostic = (level: string): void => {
    diagnostic(`XMTP SDK emitted a ${level} diagnostic`);
  };
  console.debug = () => sdkDiagnostic("debug");
  console.log = () => sdkDiagnostic("log");
  console.info = () => sdkDiagnostic("info");
  console.warn = () => sdkDiagnostic("warning");
  console.error = () => sdkDiagnostic("error");
}

function startupFailure(error: unknown): string {
  const message = error instanceof Error ? error.message.toLowerCase() : "";
  if (
    message.includes("dns") ||
    message.includes("service is currently unavailable") ||
    message.includes("network") ||
    message.includes("connect") ||
    message.includes("timeout")
  ) {
    return "could not reach XMTP; check network access, DNS, and the selected XMTP environment";
  }
  if (message.includes("permission") || message.includes("eacces")) {
    return "could not open local XMTP state; check UWUBOT_DATA_DIR ownership and permissions";
  }
  if (message.includes("database") || message.includes("sqlite")) {
    return "could not open the XMTP database; verify the persistent database key and directory";
  }
  return "startup failed; check XMTP configuration and persistent local state";
}

async function main(): Promise<void> {
  isolateProtocolOutput();
  const { Agent } = await import("@xmtp/agent-sdk");
  const identity = await prepareAgentEnvironment();
  // Agent SDK debug mode enables native structured logging on stdout. The
  // sidecar protocol owns that file descriptor, so diagnostics stay off there.
  delete process.env.XMTP_FORCE_DEBUG_LEVEL;
  const agent = await Agent.createFromEnv({
    appVersion: "cthuwu-agent/0.1.0",
    loggingLevel: "Off" as LogLevel,
  });
  const bridge = new JsonlBridge({
    input: process.stdin,
    output: process.stdout,
    timeoutMs: parseTimeout(process.env.UWUBOT_REPLY_TIMEOUT_MS),
    diagnostic,
  });

  agent.on("text", (context) => {
    if (!context.isDm()) {
      return;
    }
    const metadata = {
      messageId: context.message.id,
      senderInboxId: context.message.senderInboxId,
      // DecodedMessage metadata comes from the SDK-authenticated XMTP envelope.
      // Rust still owns role classification; the sidecar never accepts or emits a role.
      sentAtNs: context.message.sentAtNs.toString(),
      conversationId: context.message.conversationId,
    };
    const response =
      Buffer.byteLength(context.message.content, "utf8") > MAX_INBOUND_TEXT_BYTES
        ? bridge.rejectOversized(metadata)
        : bridge.request({ ...metadata, text: context.message.content });
    void response
      .then(async (response) => {
        if (response.type === "reply") {
          await context.conversation.sendText(response.text);
        }
      })
      .catch(async (_error: unknown) => {
        diagnostic("failed to process an inbound XMTP text message");
      });
  });
  agent.on("unhandledError", () => {
    diagnostic("XMTP reported an unhandled error");
  });
  agent.on("start", () => {
    diagnostic(`connected to XMTP ${identity.environment} as ${agent.address ?? "unknown"}`);
  });

  let stopping = false;
  const stop = async (reason: string): Promise<void> => {
    if (stopping) {
      return;
    }
    stopping = true;
    diagnostic(`stopping (${reason})`);
    bridge.close();
    await agent.stop();
  };
  const requestStop = (reason: string): void => {
    void stop(reason).catch(() => {
      diagnostic("graceful XMTP shutdown failed");
      process.exitCode = 1;
    });
  };
  process.once("SIGINT", () => requestStop("SIGINT"));
  process.once("SIGTERM", () => requestStop("SIGTERM"));
  process.stdin.once("end", () => requestStop("protocol input closed"));

  try {
    await agent.start();
  } catch (error) {
    bridge.close();
    await agent.stop().catch(() => undefined);
    throw error;
  }
}

void main().catch((error: unknown) => {
  diagnostic(startupFailure(error));
  process.exitCode = 1;
});
