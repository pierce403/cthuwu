import {
  Client,
  ConsentState,
  IdentifierKind,
  type Conversation,
  type DecodedMessage,
  type Signer,
} from "@xmtp/browser-sdk";
import { JsonRpcProvider, Wallet, getBytes, type HDNodeWallet } from "ethers";
import "./style.css";

const configuredAddress = (import.meta.env.VITE_XMTP_BOT_ADDRESS as string | undefined)?.trim();
const rawEnvironment = import.meta.env.VITE_XMTP_ENV as string | undefined;
const xmtpEnvironment =
  rawEnvironment === "production" || rawEnvironment === "local" ? rawEnvironment : "dev";
const encryptionKeyName = `cthuwu:${xmtpEnvironment}:db-key`;
const walletKeyName = `cthuwu:${xmtpEnvironment}:wallet-key`;

const messagesElement = requireElement<HTMLDivElement>("messages");
const composerElement = requireElement<HTMLFormElement>("composer");
const inputElement = requireElement<HTMLInputElement>("message");
const sendElement = requireElement<HTMLButtonElement>("send");
const connectElement = requireElement<HTMLButtonElement>("connect");
const statusElement = requireElement<HTMLParagraphElement>("status");

let conversation: Conversation | undefined;
let client: Client | undefined;

function requireElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing #${id}`);
  return element as T;
}

function setStatus(status: string): void {
  statusElement.textContent = status;
}

function getOrCreateEncryptionKey(): Uint8Array {
  const stored = localStorage.getItem(encryptionKeyName);
  if (stored) return getBytes(stored);

  const key = crypto.getRandomValues(new Uint8Array(32));
  localStorage.setItem(
    encryptionKeyName,
    `0x${Array.from(key, (byte) => byte.toString(16).padStart(2, "0")).join("")}`,
  );
  return key;
}

function getOrCreateWallet(): Wallet | HDNodeWallet {
  const stored = localStorage.getItem(walletKeyName);
  if (stored) return new Wallet(stored);

  const wallet = Wallet.createRandom();
  localStorage.setItem(walletKeyName, wallet.privateKey);
  return wallet;
}

function createSigner(wallet: Wallet | HDNodeWallet): Signer {
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
  if (!resolved) throw new Error(`Could not resolve ${address}`);
  return resolved.toLowerCase();
}

function renderMessage(message: DecodedMessage): void {
  if (message.contentType?.typeId !== "text") return;

  const bubble = document.createElement("article");
  bubble.className = `message ${message.senderInboxId === client?.inboxId ? "mine" : "theirs"}`;
  bubble.textContent = String(message.content);
  messagesElement.append(bubble);
  messagesElement.scrollTop = messagesElement.scrollHeight;
}

async function streamMessages(activeConversation: Conversation): Promise<void> {
  const stream = await activeConversation.stream();
  for await (const message of stream as unknown as AsyncIterable<DecodedMessage>) {
    if (message) renderMessage(message);
  }
}

async function connect(): Promise<void> {
  if (!configuredAddress) {
    setStatus("set VITE_XMTP_BOT_ADDRESS before building");
    return;
  }

  connectElement.disabled = true;
  setStatus(`opening the ${xmtpEnvironment} portal…`);

  try {
    const wallet = getOrCreateWallet();
    client = await Client.create(createSigner(wallet), {
      env: xmtpEnvironment,
      dbEncryptionKey: getOrCreateEncryptionKey(),
    });

    const address = await resolveCompanionAddress(configuredAddress);
    conversation = await client.conversations.createDmWithIdentifier({
      identifier: address,
      identifierKind: IdentifierKind.Ethereum,
    });

    if ((await conversation.consentState()) !== ConsentState.Allowed) {
      await conversation.updateConsentState(ConsentState.Allowed);
    }

    messagesElement.replaceChildren();
    for (const message of await conversation.messages()) renderMessage(message);

    inputElement.disabled = false;
    sendElement.disabled = false;
    connectElement.hidden = true;
    inputElement.focus();
    setStatus("portal open · this browser identity is stored on this device");
    void streamMessages(conversation);
  } catch (error) {
    console.error(error);
    setStatus(error instanceof Error ? error.message : "the portal would not open");
    connectElement.disabled = false;
  }
}

connectElement.addEventListener("click", () => void connect());

// First visit creates a dedicated local identity; returning visits reuse it.
void connect();

composerElement.addEventListener("submit", (event) => {
  event.preventDefault();
  const text = inputElement.value.trim();
  if (!conversation || !text) return;

  inputElement.value = "";
  void conversation.sendText(text).catch((error: unknown) => {
    console.error(error);
    setStatus(error instanceof Error ? error.message : "message failed");
  });
});
