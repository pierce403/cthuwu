import { parseConfig, parseEnvironment } from "./config";
import {
  IdentityStorageError,
  loadOrCreateIdentity,
  persistIdentity,
  resetIdentity,
  type StoredIdentity,
} from "./identity";
import { decryptIdentityBackup, encryptIdentityBackup } from "./identity-backup";
import type { ChatMessage, ChatSession } from "./transport";
import { createXmtpSession } from "./xmtp-transport";
import "./style.css";

const MAX_MESSAGE_BYTES = 16 * 1024;
const MAX_RENDERED_MESSAGES = 200;
const environment = parseEnvironment(import.meta.env.VITE_XMTP_ENV as string | undefined);

const chatElement = requireElement<HTMLElement>("chat");
const messagesElement = requireElement<HTMLDivElement>("messages");
const composerElement = requireElement<HTMLFormElement>("composer");
const inputElement = requireElement<HTMLInputElement>("message");
const sendElement = requireElement<HTMLButtonElement>("send");
const connectElement = requireElement<HTMLButtonElement>("connect");
const settingsElement = requireElement<HTMLButtonElement>("settings");
const statusElement = requireElement<HTMLParagraphElement>("status");
const composerErrorElement = requireElement<HTMLParagraphElement>("composer-error");
const dialogElement = requireElement<HTMLDialogElement>("identity-dialog");
const addressElement = requireElement<HTMLElement>("identity-address");
const environmentElement = requireElement<HTMLElement>("identity-environment");
const passphraseElement = requireElement<HTMLInputElement>("backup-passphrase");
const exportElement = requireElement<HTMLButtonElement>("export-identity");
const importElement = requireElement<HTMLInputElement>("import-identity");
const resetElement = requireElement<HTMLButtonElement>("reset-identity");
const settingsStatusElement = requireElement<HTMLParagraphElement>("settings-status");

let identity: StoredIdentity | undefined;
let session: ChatSession | undefined;
let stopStream: (() => Promise<void>) | undefined;
let connectionPromise: Promise<void> | undefined;
let connectionGeneration = 0;
let sending = false;
const seenMessageIds = new Set<string>();

bootstrap();

function bootstrap(): void {
  environmentElement.textContent = environment;
  wireIdentityControls();
  try {
    // Identity creation happens before bot configuration or network access.
    identity = loadOrCreateIdentity(environment);
    addressElement.textContent = identity.address;
  } catch (error) {
    fatalIdentity(error);
    return;
  }
  void connect(false);
}

async function connect(userInitiated: boolean): Promise<void> {
  if (connectionPromise) return connectionPromise;
  const generation = ++connectionGeneration;
  connectionPromise = (async () => {
    setConnectionStatus("opening the XMTP portal…", true);
    connectElement.hidden = true;
    try {
      if (!identity) throw new Error("browser identity unavailable");
      const config = parseConfig(
        environment,
        import.meta.env.VITE_XMTP_BOT_ADDRESS as string | undefined,
      );
      await closeSession();
      const nextSession = await createXmtpSession(config, identity);
      if (generation !== connectionGeneration) {
        await nextSession.close();
        return;
      }
      session = nextSession;
      seenMessageIds.clear();
      messagesElement.replaceChildren();
      stopStream = await session.stream(renderMessage, streamFailed);
      for (const message of await session.history()) renderMessage(message, false);
      inputElement.disabled = false;
      sendElement.disabled = false;
      chatElement.setAttribute("aria-busy", "false");
      setConnectionStatus(
        `portal open · inbox ${shortId(session.inboxId)} · identity stored on this device`,
        false,
      );
      if (userInitiated) inputElement.focus();
    } catch (error) {
      console.error(error);
      setConnectionStatus("the XMTP portal could not open; your local identity was preserved", false);
      connectElement.hidden = false;
      connectElement.disabled = false;
      chatElement.setAttribute("aria-busy", "false");
    } finally {
      connectionPromise = undefined;
    }
  })();
  return connectionPromise;
}

function renderMessage(message: ChatMessage, announce = true): void {
  if (seenMessageIds.has(message.id)) return;
  seenMessageIds.add(message.id);
  const bubble = document.createElement("article");
  bubble.className = `message ${message.mine ? "mine" : "theirs"}`;
  const sender = document.createElement("span");
  sender.className = "sender";
  sender.textContent = message.mine ? "You" : "Cthuwu";
  bubble.append(sender, document.createTextNode(message.text));
  if (!announce) bubble.setAttribute("aria-live", "off");
  messagesElement.append(bubble);
  while (messagesElement.childElementCount > MAX_RENDERED_MESSAGES) {
    messagesElement.firstElementChild?.remove();
  }
  messagesElement.scrollTop = messagesElement.scrollHeight;
}

function streamFailed(error: unknown): void {
  console.error(error);
  setConnectionStatus("the message stream closed; retry the portal to catch up", false);
  connectElement.hidden = false;
}

composerElement.addEventListener("submit", (event) => {
  event.preventDefault();
  void sendCurrentDraft();
});

async function sendCurrentDraft(): Promise<void> {
  const draft = inputElement.value.trim();
  if (!session || !draft || sending) return;
  if (new TextEncoder().encode(draft).length > MAX_MESSAGE_BYTES) {
    setComposerError("Messages must be 16 KiB or smaller.");
    return;
  }
  sending = true;
  sendElement.disabled = true;
  setComposerError();
  try {
    await session.send(draft);
    if (inputElement.value.trim() === draft) inputElement.value = "";
  } catch (error) {
    console.error(error);
    setComposerError("The send was not confirmed. Your draft is still here; check before retrying.");
  } finally {
    sending = false;
    sendElement.disabled = false;
  }
}

connectElement.addEventListener("click", () => void connect(true));
settingsElement.addEventListener("click", () => dialogElement.showModal());

function wireIdentityControls(): void {
  exportElement.addEventListener("click", () => void exportIdentity());
  importElement.addEventListener("change", () => void importIdentity());
  resetElement.addEventListener("click", () => void confirmReset());
}

async function exportIdentity(): Promise<void> {
  try {
    if (!identity) throw new Error("No valid identity is loaded");
    const backup = await encryptIdentityBackup(identity, passphraseElement.value);
    const blob = new Blob([backup], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `cthuwu-${environment}-identity.json`;
    link.click();
    URL.revokeObjectURL(url);
    setSettingsStatus("encrypted identity backup downloaded");
  } catch (error) {
    setSettingsStatus(publicError(error));
  }
}

async function importIdentity(): Promise<void> {
  const file = importElement.files?.[0];
  if (!file) return;
  try {
    const restored = await decryptIdentityBackup(
      await file.text(),
      passphraseElement.value,
      environment,
    );
    persistIdentity(restored);
    setSettingsStatus("identity restored; reloading…");
    location.reload();
  } catch (error) {
    setSettingsStatus(publicError(error));
    importElement.value = "";
  }
}

async function confirmReset(): Promise<void> {
  const confirmed = window.confirm(
    "Reset this identity? Without an exported backup, the old XMTP inbox may become inaccessible. Reset does not securely erase the Browser SDK's unencrypted local message database; clear this site's browser data for full local deletion.",
  );
  if (!confirmed) return;
  await closeSession();
  resetIdentity(environment);
  location.reload();
}

async function closeSession(): Promise<void> {
  if (stopStream) {
    await stopStream().catch(() => undefined);
    stopStream = undefined;
  }
  if (session) {
    await session.close().catch(() => undefined);
    session = undefined;
  }
}

window.addEventListener("pagehide", () => void closeSession());

function fatalIdentity(error: unknown): void {
  console.error(error);
  const message =
    error instanceof IdentityStorageError
      ? error.message
      : "The browser identity could not be created or loaded";
  setConnectionStatus(message, false);
  chatElement.setAttribute("aria-busy", "false");
  dialogElement.showModal();
}

function setConnectionStatus(message: string, busy: boolean): void {
  statusElement.textContent = message;
  chatElement.setAttribute("aria-busy", String(busy));
}

function setComposerError(message?: string): void {
  composerErrorElement.hidden = !message;
  composerErrorElement.textContent = message ?? "";
}

function setSettingsStatus(message: string): void {
  settingsStatusElement.textContent = message;
}

function publicError(error: unknown): string {
  return error instanceof Error ? error.message : "The identity operation failed";
}

function shortId(value: string): string {
  return value.length > 14 ? `${value.slice(0, 7)}…${value.slice(-6)}` : value;
}

function requireElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing #${id}`);
  return element as T;
}
