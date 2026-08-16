import type { LogLevel } from "@xmtp/agent-sdk";
import path from "node:path";
import {
  AssignmentCodec,
  CanonicalAssignmentResolver,
  ChatControlService,
  FileChatStateStore,
  JoinCodec,
  LivenessControlGate,
  LivenessJoinCodec,
  LivenessQueryCodec,
  LivenessResponseCodec,
  TypingCodec,
  bootstrapGlobalGroup,
  classifyInboundMessage,
  dispatchPersonalText,
  handleInboundChatControl,
  loadVerifiedRegistration,
  parseChatControlConfig,
  parseGlobalAdminInboxIds,
  resolveFreshSenderAddress,
  type GroupDirectory,
  type GroupLike,
  type TypingControl,
} from "./chat-control.js";
import { catchUpDirectMessages, type CatchUpDm, type CatchUpMessage } from "./catch-up.js";
import { runErc8004Stdio } from "./erc8004.js";
import { loadAgentIdentity } from "./identity.js";
import { resolveOperatorIdentity } from "./operator-identity.js";
import { JsonlBridge, parseTimeout } from "./protocol.js";

const MAX_INBOUND_TEXT_BYTES = 16 * 1024;
const TYPING_TTL_MS = 15_000;
const TYPING_REFRESH_MS = 8_000;

function diagnostic(message: string): void {
  process.stderr.write(`[cthuwu-xmtp] ${message.replace(/[\r\n]+/gu, " ")}\n`);
}

function diagnosticError(error: unknown, fallback: string): string {
  const message = error instanceof Error ? error.message : fallback;
  return message.replace(/[\r\n]+/gu, " ").slice(0, 512);
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
  if (process.argv.includes("--erc8004")) {
    if (
      process.env.XMTP_ENV !== "production" ||
      process.env.UWUBOT_XMTP_ENV !== "production"
    ) {
      throw new Error(
        "ERC-8004 helper requires the persistent XMTP production identity",
      );
    }
    await runErc8004Stdio(process.stdin, process.stdout, () =>
      loadAgentIdentity(process.env),
    );
    return;
  }
  const resolveOperatorIndex = process.argv.indexOf("--resolve-operator-inbox");
  if (resolveOperatorIndex !== -1) {
    const operatorIdentity = process.argv[resolveOperatorIndex + 1];
    if (operatorIdentity === undefined) {
      throw new Error("--resolve-operator-inbox requires an ENS name or Ethereum address");
    }
    const environment = process.env.XMTP_ENV;
    if (environment !== "production" && environment !== "dev" && environment !== "local") {
      throw new Error("XMTP_ENV must be production, dev, or local");
    }
    try {
      const resolved = await resolveOperatorIdentity(operatorIdentity, environment);
      process.stdout.write(
        `${JSON.stringify({ type: "operator_identity", ...resolved })}\n`,
      );
    } catch (error: unknown) {
      const message =
        error instanceof Error ? error.message : "operator identity resolution failed";
      process.stdout.write(
        `${JSON.stringify({
          type: "operator_identity_error",
          message: message.replace(/[\r\n]+/gu, " ").slice(0, 512),
        })}\n`,
      );
    }
    return;
  }
  const identity = await loadAgentIdentity();
  if (process.argv.includes("--print-xmtp-wallet-address")) {
    process.stdout.write(
      `${JSON.stringify({
        type: "xmtp_identity",
        walletAddress: identity.walletAddress,
      })}\n`,
    );
    return;
  }
  const { Agent, Group, createSigner, createUser } = await import("@xmtp/agent-sdk");
  // Agent SDK debug mode enables native structured logging on stdout. The
  // sidecar protocol owns that file descriptor, so diagnostics stay off there.
  delete process.env.XMTP_FORCE_DEBUG_LEVEL;
  const agent = await Agent.create(createSigner(createUser(identity.walletKey)), {
    env: identity.environment,
    dbEncryptionKey: identity.dbEncryptionKey,
    dbPath: (inboxId) => path.join(identity.dbDirectory, `xmtp-${inboxId}.db3`),
    ...(process.env.XMTP_GATEWAY_HOST === undefined
      ? {}
      : { gatewayHost: process.env.XMTP_GATEWAY_HOST }),
    appVersion: "cthuwu-agent/0.1.0",
    loggingLevel: "Off" as LogLevel,
    codecs: [
      new JoinCodec(),
      new AssignmentCodec(),
      new TypingCodec(),
      new LivenessQueryCodec(),
      new LivenessResponseCodec(),
      new LivenessJoinCodec(),
    ],
  });

  const groupDirectory: GroupDirectory = {
    sync: async () => agent.client.conversations.sync(),
    listGroups: () =>
      agent.client.conversations.listGroups() as unknown as GroupLike[],
    getConversationById: async (id) => {
      const conversation = await agent.client.conversations.getConversationById(id);
      return conversation instanceof Group
        ? (conversation as unknown as GroupLike)
        : undefined;
    },
    createGroup: async (inboxIds, options) =>
      (await agent.client.conversations.createGroup(
        inboxIds,
        options,
      )) as unknown as GroupLike,
  };

  const bootstrapIndex = process.argv.indexOf("--global-group-bootstrap");
  if (bootstrapIndex !== -1) {
    const action = process.argv[bootstrapIndex + 1];
    if (action !== "create" && action !== "inspect") {
      throw new Error("--global-group-bootstrap requires create or inspect");
    }
    if (identity.environment !== "production") {
      throw new Error("Global group administration requires XMTP production");
    }
    const selfInboxId = agent.client.inboxId;
    const registration = await loadVerifiedRegistration(
      process.env.UWUBOT_DATA_DIR ?? ".",
      identity.walletAddress,
      selfInboxId,
    );
    const store = new FileChatStateStore(
      process.env.UWUBOT_DATA_DIR ?? ".",
      registration.agentId,
      selfInboxId,
    );
    const state = await store.load();
    const configuredGroupId = process.env.CTHUWU_GLOBAL_GROUP_ID;
    if (
      state.globalGroupId !== undefined &&
      configuredGroupId !== state.globalGroupId
    ) {
      throw new Error(
        "persisted Global binding must match CTHUWU_GLOBAL_GROUP_ID before administration",
      );
    }
    const result = await bootstrapGlobalGroup({
      action,
      directory: groupDirectory,
      selfInboxId,
      adminInboxIds: parseGlobalAdminInboxIds(
        process.env.CTHUWU_GLOBAL_ADMIN_INBOX_IDS,
        selfInboxId,
      ),
      ...(configuredGroupId === undefined ? {} : { configuredGroupId }),
    });
    state.globalGroupId = result.groupId;
    await store.save(state);
    process.stdout.write(`${JSON.stringify({ type: "cthuwu_global_group", ...result })}\n`);
    await agent.stop();
    return;
  }

  let chatControl: ChatControlService | undefined;
  const livenessGate = new LivenessControlGate();
  let localTentacleAgentId: string | undefined;
  let chatRevalidateSeconds = 900;
  if (identity.environment === "production") {
    let localRegistration: Awaited<ReturnType<typeof loadVerifiedRegistration>> | undefined;
    try {
      const selfInboxId = agent.client.inboxId;
      localRegistration = await loadVerifiedRegistration(
        process.env.UWUBOT_DATA_DIR ?? ".",
        identity.walletAddress,
        selfInboxId,
      );
      localTentacleAgentId = localRegistration.agentId;
      diagnostic(
        `enabled authenticated liveness responses for ERC-8004 agent ${localRegistration.agentId}`,
      );
    } catch (error: unknown) {
      diagnostic(
        `liveness responses are disabled because the local ERC-8004 registration is unavailable: ${diagnosticError(error, "registration verification failed")}`,
      );
    }
    if (localRegistration !== undefined) {
      try {
        const selfInboxId = agent.client.inboxId;
        const config = parseChatControlConfig(process.env, selfInboxId);
        chatRevalidateSeconds = config.assignmentRevalidateSeconds;
        chatControl = new ChatControlService({
          directory: groupDirectory,
          store: new FileChatStateStore(
            process.env.UWUBOT_DATA_DIR ?? ".",
            localRegistration.agentId,
            selfInboxId,
          ),
          resolver: new CanonicalAssignmentResolver({
            ...(process.env.CTHUWU_RPC_ENDPOINT === undefined
              ? {}
              : { rpcEndpoint: process.env.CTHUWU_RPC_ENDPOINT }),
            ...(process.env.CTHUWU_BRANDING_CONTRACT === undefined
              ? {}
              : { brandingContract: process.env.CTHUWU_BRANDING_CONTRACT }),
            localRegistration,
          }),
          config,
          selfInboxId,
          tentacleAgentId: localRegistration.agentId,
          livenessGate,
          resolveInboxAddress: async (inboxId) =>
            resolveFreshSenderAddress(agent.client.preferences, inboxId),
        });
        diagnostic("enabled authenticated three-channel XMTP enrollment");
      } catch (error: unknown) {
        diagnostic(
          `three-channel XMTP enrollment is disabled: ${diagnosticError(error, "enrollment configuration failed")}; Direct messaging and liveness responses remain available`,
        );
      }
    }
  }

  const typingCodec = new TypingCodec();
  agent.use(async (context, next) => {
    const contentType = context.message.contentType;
    const disposition = classifyInboundMessage(context.isDm(), contentType);
    if (disposition !== "control") {
      await next();
      return;
    }
    // Every control type terminates here. It never enters text events, Rust, inference, contact
    // memory, or ordinary history, even when malformed, forged, or delivered in a group.
    try {
      const result = await handleInboundChatControl({
        isDm: context.isDm(),
        contentType,
        content: context.message.content,
        senderInboxId: context.message.senderInboxId,
        conversation: context.conversation,
        livenessGate,
        resolveSenderAddress: () => resolveFreshSenderAddress(
          context.client.preferences,
          context.message.senderInboxId,
        ),
        ...(localTentacleAgentId === undefined ? {} : { localTentacleAgentId }),
        ...(chatControl === undefined ? {} : { chatControl }),
      });
      switch (result.kind) {
        case "liveness-sender-unresolved":
          diagnostic("ignored a liveness query because its authenticated XMTP inbox has no unique current EVM address");
          break;
        case "liveness-response-sent":
          diagnostic(`answered a liveness query for ERC-8004 agent ${localTentacleAgentId ?? "unverified"}`);
          break;
        case "liveness-response-failed":
          diagnostic("failed to send a liveness response; revoked its one-use enrollment grant");
          break;
        case "assignment-sent":
          diagnostic("delivered an authenticated three-channel XMTP assignment");
          break;
        case "liveness-unavailable":
        case "liveness-target-mismatch":
        case "enrollment-unavailable":
        case "enrollment-sender-unresolved":
        case "enrollment-no-assignment":
        case "ignored":
          break;
      }
    } catch (error: unknown) {
      diagnostic(
        `failed to process an authenticated XMTP control: ${diagnosticError(error, "control processing failed")}`,
      );
    }
  });
  const bridge = new JsonlBridge({
    input: process.stdin,
    output: process.stdout,
    timeoutMs: parseTimeout(process.env.UWUBOT_REPLY_TIMEOUT_MS),
    diagnostic,
    onOperatorNotice: async ({ inboxId, text }) => {
      const conversation = await agent.client.conversations.createDm(inboxId);
      await conversation.sendText(text);
      diagnostic("delivered an ERC-8004 notice to an authenticated operator inbox");
    },
  });

  const processDirectText = async (
    message: CatchUpMessage,
    conversation: CatchUpDm,
    senderAddress: string | undefined,
    quietReplay = false,
  ): Promise<void> => {
    if (typeof message.content !== "string") {
      return;
    }
    let text: string | undefined;
    dispatchPersonalText(true, message.contentType, message.content, (value) => {
      text = value;
    });
    if (text === undefined) {
      return;
    }
    try {
      const metadata = {
        messageId: message.id,
        senderInboxId: message.senderInboxId,
        ...(senderAddress === undefined ? {} : { senderAddress }),
        sentAtNs: message.sentAtNs.toString(),
        conversationId: message.conversationId,
      };
      const inboundBytes = Buffer.byteLength(text, "utf8");
      if (!quietReplay) {
        diagnostic(`received direct XMTP message (${inboundBytes} bytes); waiting for uwubot`);
      }
      const sendTyping = async (active: boolean): Promise<void> => {
        const value: TypingControl = {
          type: "cthuwu.typing.v1",
          active,
          expiresAtNs: (BigInt(Date.now() + (active ? TYPING_TTL_MS : 1)) * 1_000_000n).toString(),
        };
        await conversation.send(typingCodec.encode(value), { shouldPush: false });
      };
      if (!quietReplay) {
        await sendTyping(true).catch(() => diagnostic("failed to start XMTP typing indicator"));
      }
      const refresh = quietReplay ? undefined : setInterval(() => {
        void sendTyping(true).catch(() => diagnostic("failed to refresh XMTP typing indicator"));
      }, TYPING_REFRESH_MS);
      let result;
      try {
        result = await (inboundBytes > MAX_INBOUND_TEXT_BYTES
          ? bridge.rejectOversized(metadata)
          : bridge.request({ ...metadata, text }));
      } finally {
        if (refresh) clearInterval(refresh);
        if (!quietReplay) {
          await sendTyping(false).catch(() => diagnostic("failed to clear XMTP typing indicator"));
        }
      }
      if (result.type === "reply") {
        diagnostic("uwubot finished; delivering XMTP reply");
        await conversation.sendText(result.text);
        diagnostic("delivered XMTP reply");
      } else {
        if (!quietReplay) {
          diagnostic("uwubot ignored the XMTP message");
        }
      }
    } catch (_error: unknown) {
      diagnostic("failed to process an inbound XMTP text message");
    }
  };

  agent.on("text", (context) => {
    if (context.isDm()) {
      void (async () => {
        // This identifier is resolved from the SDK-authenticated sender inbox. It is optional
        // because XMTP inboxes may use identifier types that are not EVM addresses. A resolver
        // failure must not drop an otherwise valid, transport-authenticated XMTP message.
        let senderAddress: string | undefined;
        try {
          senderAddress = await context.getSenderAddress();
        } catch (_error: unknown) {
          senderAddress = undefined;
        }
        await processDirectText(context.message, context.conversation, senderAddress);
      })().catch(() => diagnostic("failed to prepare an inbound XMTP text message"));
    }
  });
  const pruneMovedAssignments = (): void => {
    void chatControl?.pruneMovedAssignments().catch(() => {
      diagnostic("Acolytes reassignment sweep failed closed");
    });
  };
  agent.on("unhandledError", () => {
    diagnostic("XMTP reported an unhandled error");
  });
  agent.on("start", () => {
    diagnostic(`connected to XMTP ${identity.environment} as ${agent.address ?? "unknown"}`);
    pruneMovedAssignments();
    void catchUpDirectMessages({
      conversations: agent.client.conversations,
      selfInboxId: agent.client.inboxId,
      handle: async (message, conversation) => {
        const senderAddress = await resolveFreshSenderAddress(
          agent.client.preferences,
          message.senderInboxId,
        ).catch(() => undefined);
        await processDirectText(message, conversation, senderAddress, true);
      },
    })
      .then((result) => {
        diagnostic(
          `startup DM catch-up checked ${result.conversations} conversations and ${result.messages} inbound messages${result.truncated ? "; bounded history was truncated" : ""}`,
        );
      })
      .catch(() => diagnostic("startup DM catch-up failed; live streaming remains active"));
  });

  let stopping = false;
  let pruneTimer: NodeJS.Timeout | undefined;
  if (chatControl !== undefined) {
    pruneTimer = setInterval(pruneMovedAssignments, chatRevalidateSeconds * 1000);
    pruneTimer.unref();
  }
  const stop = async (reason: string): Promise<void> => {
    if (stopping) {
      return;
    }
    stopping = true;
    diagnostic(`stopping (${reason})`);
    if (pruneTimer !== undefined) {
      clearInterval(pruneTimer);
    }
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
