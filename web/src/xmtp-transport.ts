import {
  Client,
  ConsentState,
  IdentifierKind,
  type Conversation,
  type DecodedMessage,
  type Signer,
} from "@xmtp/browser-sdk";
import { JsonRpcProvider, Wallet, getBytes } from "ethers";
import type { AppConfig } from "./config";
import type { StoredIdentity } from "./identity";
import type { ChatMessage, ChatSession } from "./transport";

export async function createXmtpSession(
  config: AppConfig,
  identity: StoredIdentity,
): Promise<ChatSession> {
  const wallet = new Wallet(identity.walletPrivateKey);
  const client = await Client.create(createSigner(wallet), {
    env: config.environment,
    appVersion: "cthuwu-web/0.1.0",
  });
  const botAddress = await resolveCompanionAddress(config.botAddress);
  const conversation = await client.conversations.createDmWithIdentifier({
    identifier: botAddress,
    identifierKind: IdentifierKind.Ethereum,
  });
  if ((await conversation.consentState()) !== ConsentState.Allowed) {
    await conversation.updateConsentState(ConsentState.Allowed);
  }
  return new XmtpSession(client, conversation);
}

class XmtpSession implements ChatSession {
  readonly inboxId: string;
  private closed = false;

  constructor(
    private readonly client: Client,
    private readonly conversation: Conversation,
  ) {
    if (!client.inboxId) throw new Error("XMTP client did not return an inbox ID");
    this.inboxId = client.inboxId;
  }

  async history(): Promise<ChatMessage[]> {
    return (await this.conversation.messages()).flatMap((message) => {
      const decoded = decodeMessage(message, this.inboxId);
      return decoded ? [decoded] : [];
    });
  }

  async stream(
    onMessage: (message: ChatMessage) => void,
    onError: (error: unknown) => void,
  ): Promise<() => Promise<void>> {
    const stream = await this.conversation.stream();
    void (async () => {
      try {
        for await (const message of stream as unknown as AsyncIterable<DecodedMessage>) {
          if (this.closed || !message) break;
          const decoded = decodeMessage(message, this.inboxId);
          if (decoded) onMessage(decoded);
        }
      } catch (error) {
        if (!this.closed) onError(error);
      }
    })();
    return async () => {
      this.closed = true;
      await stream.return?.();
    };
  }

  async send(text: string): Promise<void> {
    await this.conversation.sendText(text);
  }

  async close(): Promise<void> {
    this.closed = true;
    await this.client.close();
  }
}

function decodeMessage(message: DecodedMessage, ownInboxId: string): ChatMessage | undefined {
  if (message.contentType?.typeId !== "text") return undefined;
  return {
    id: String(message.id),
    text: String(message.content),
    mine: message.senderInboxId === ownInboxId,
  };
}

function createSigner(wallet: Wallet): Signer {
  return {
    type: "EOA",
    getIdentifier: () => ({
      identifier: wallet.address.toLowerCase(),
      identifierKind: IdentifierKind.Ethereum,
    }),
    signMessage: async (message: string) => getBytes(await wallet.signMessage(message)),
  };
}

async function resolveCompanionAddress(address: string): Promise<string> {
  if (!address.endsWith(".eth")) return address.toLowerCase();
  const resolved = await new JsonRpcProvider("https://cloudflare-eth.com").resolveName(address);
  if (!resolved) throw new Error("Cthuwu's ENS name did not resolve");
  return resolved.toLowerCase();
}
