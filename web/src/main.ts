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
const MOTION_STORAGE_KEY = "cthuwu.ui.motion.v1";
const environment = parseEnvironment(import.meta.env.VITE_XMTP_ENV as string | undefined);

type UiState =
  | "preparing"
  | "connecting"
  | "connected"
  | "sending"
  | "retryable-error"
  | "fatal-error";

const chatElement = requireElement<HTMLElement>("chat");
const mascotElement = requireElement<HTMLElement>("mascot-stage");
const messagesElement = requireElement<HTMLDivElement>("messages");
const newMessagesElement = requireElement<HTMLButtonElement>("new-messages");
const composerElement = requireElement<HTMLFormElement>("composer");
const inputElement = requireElement<HTMLTextAreaElement>("message");
const sendElement = requireElement<HTMLButtonElement>("send");
const sendLabelElement = sendElement.querySelector("span");
const connectElement = requireElement<HTMLButtonElement>("connect");
const settingsElement = requireElement<HTMLButtonElement>("settings");
const statusElement = requireElement<HTMLParagraphElement>("status");
const composerErrorElement = requireElement<HTMLParagraphElement>("composer-error");
const motionElement = requireElement<HTMLButtonElement>("motion-toggle");
const motionLabelElement = motionElement.querySelector("span");
const dialogElement = requireElement<HTMLDialogElement>("identity-dialog");
const dialogCloseElement = requireElement<HTMLButtonElement>("identity-close");
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
let uiState: UiState = "preparing";
let connectedStatus = "XMTP connection ready";
let sending = false;
let delightTimer: ReturnType<typeof setTimeout> | undefined;
const seenMessageIds = new Set<string>();

bootstrap();

function bootstrap(): void {
  environmentElement.textContent = environment;
  wireIdentityControls();
  wireComposerControls();
  initializeMotionControl();
  setUiState("preparing", "preparing a local identity…");
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
    setUiState("connecting", "parting the veil through XMTP…");
    let nextSession: ChatSession | undefined;
    try {
      if (!identity) throw new Error("browser identity unavailable");
      const config = parseConfig(
        environment,
        import.meta.env.VITE_XMTP_BOT_ADDRESS as string | undefined,
      );
      await closeSession();
      nextSession = await createXmtpSession(config, identity);
      if (generation !== connectionGeneration) {
        await nextSession.close();
        return;
      }

      session = nextSession;
      seenMessageIds.clear();
      messagesElement.replaceChildren();
      messagesElement.classList.remove("is-empty");
      newMessagesElement.hidden = true;
      stopStream = await session.stream(
        (message) => renderMessage(message),
        (error) => streamFailed(error, generation),
      );
      for (const message of await session.history()) renderMessage(message, false);
      if (messagesElement.childElementCount === 0) renderWelcome();

      connectedStatus = `veil open · inbox ${shortId(session.inboxId)} · local browser identity`;
      setUiState("connected", connectedStatus);
      if (userInitiated) inputElement.focus();
    } catch (error) {
      console.error(error);
      if (nextSession && session !== nextSession) await nextSession.close().catch(() => undefined);
      if (generation === connectionGeneration) {
        connectionGeneration += 1;
        setUiState(
          "retryable-error",
          "the XMTP veil got tangled; your local identity is still here",
        );
        await closeSession();
      }
    } finally {
      connectionPromise = undefined;
    }
  })();
  return connectionPromise;
}

function renderMessage(message: ChatMessage, announce = true): void {
  if (seenMessageIds.has(message.id)) return;
  seenMessageIds.add(message.id);

  const shouldFollow = message.mine || isNearMessageBottom();
  messagesElement.querySelector("[data-welcome]")?.remove();
  messagesElement.classList.remove("is-empty");

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

  if (shouldFollow) {
    scrollToLatest();
  } else if (announce) {
    newMessagesElement.hidden = false;
  }
  if (announce && !message.mine) delightMascot();
}

function renderWelcome(): void {
  const welcome = document.createElement("div");
  welcome.className = "welcome";
  welcome.dataset.welcome = "";
  welcome.setAttribute("role", "note");

  const spark = document.createElement("span");
  spark.className = "welcome-spark";
  spark.setAttribute("aria-hidden", "true");
  spark.textContent = "✦";

  const kicker = document.createElement("p");
  kicker.className = "welcome-kicker";
  kicker.textContent = "a warm corner of the void";

  const greeting = document.createElement("p");
  greeting.textContent = "oh! you made it. i was hoping the stars would send someone interesting.";

  const question = document.createElement("p");
  question.className = "welcome-question";
  question.textContent = "what’s on your mind?";

  welcome.append(spark, kicker, greeting, question);
  messagesElement.classList.add("is-empty");
  messagesElement.append(welcome);
}

function streamFailed(error: unknown, generation: number): void {
  if (generation !== connectionGeneration) return;
  console.error(error);
  connectionGeneration += 1;
  void disconnectAfterStreamFailure();
}

async function disconnectAfterStreamFailure(): Promise<void> {
  setUiState("retryable-error", "the message stream closed; reconnect to catch up safely");
  await closeSession();
}

function wireComposerControls(): void {
  composerElement.addEventListener("submit", (event) => {
    event.preventDefault();
    void sendCurrentDraft();
  });
  inputElement.addEventListener("input", () => {
    resizeComposer();
    setComposerError();
    updateSendAvailability();
  });
  inputElement.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    composerElement.requestSubmit();
  });
  messagesElement.addEventListener("scroll", () => {
    if (isNearMessageBottom()) newMessagesElement.hidden = true;
  });
  newMessagesElement.addEventListener("click", scrollToLatest);
  connectElement.addEventListener("click", () => void connect(true));
  settingsElement.addEventListener("click", () => dialogElement.showModal());
}

async function sendCurrentDraft(): Promise<void> {
  const draft = inputElement.value.trim();
  if (!session || uiState !== "connected" || !draft || sending) return;
  if (new TextEncoder().encode(draft).length > MAX_MESSAGE_BYTES) {
    setComposerError("Messages must be 16 KiB or smaller.");
    return;
  }

  const activeSession = session;
  sending = true;
  setComposerError();
  setUiState("sending", "carrying your whisper through XMTP…");
  try {
    await activeSession.send(draft);
    if (inputElement.value.trim() === draft) {
      inputElement.value = "";
      resizeComposer();
    }
  } catch (error) {
    console.error(error);
    setComposerError("The send was not confirmed. Your draft is still here; check before retrying.");
  } finally {
    sending = false;
    if (session === activeSession) {
      setUiState("connected", connectedStatus);
    } else {
      updateSendAvailability();
    }
  }
}

function wireIdentityControls(): void {
  dialogElement.querySelector("form")?.addEventListener("submit", (event) => event.preventDefault());
  dialogCloseElement.addEventListener("click", () => dialogElement.close());
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
  const activeStop = stopStream;
  const activeSession = session;
  stopStream = undefined;
  session = undefined;
  if (activeStop) await activeStop().catch(() => undefined);
  if (activeSession) await activeSession.close().catch(() => undefined);
}

window.addEventListener("pagehide", () => void closeSession());

function fatalIdentity(error: unknown): void {
  console.error(error);
  const message =
    error instanceof IdentityStorageError
      ? error.message
      : "The browser identity could not be created or loaded";
  setUiState("fatal-error", message);
  dialogElement.showModal();
}

function setUiState(state: UiState, message: string): void {
  uiState = state;
  chatElement.dataset.state = state;
  mascotElement.dataset.mood = mascotMood(state);
  messagesElement.setAttribute(
    "aria-busy",
    String(state === "preparing" || state === "connecting"),
  );
  statusElement.textContent = message;

  const composerReady = state === "connected" || state === "sending";
  inputElement.disabled = !composerReady;
  connectElement.hidden = state !== "retryable-error";
  connectElement.disabled = state === "connecting";
  if (sendLabelElement) sendLabelElement.textContent = state === "sending" ? "sending…" : "whisper";
  updateSendAvailability();
}

function mascotMood(state: UiState): string {
  switch (state) {
    case "preparing":
    case "connecting":
      return "summoning";
    case "connected":
      return "happy";
    case "sending":
      return "listening";
    case "retryable-error":
    case "fatal-error":
      return "worried";
  }
}

function updateSendAvailability(): void {
  sendElement.disabled = uiState !== "connected" || sending || inputElement.value.trim().length === 0;
}

function setComposerError(message?: string): void {
  composerErrorElement.hidden = !message;
  composerErrorElement.textContent = message ?? "";
  inputElement.setAttribute("aria-invalid", String(Boolean(message)));
}

function setSettingsStatus(message: string): void {
  settingsStatusElement.textContent = message;
}

function resizeComposer(): void {
  inputElement.style.height = "auto";
  inputElement.style.height = `${Math.min(inputElement.scrollHeight, 136)}px`;
  inputElement.style.overflowY = inputElement.scrollHeight > 136 ? "auto" : "hidden";
}

function isNearMessageBottom(): boolean {
  return messagesElement.scrollHeight - messagesElement.scrollTop - messagesElement.clientHeight < 80;
}

function scrollToLatest(): void {
  messagesElement.scrollTop = messagesElement.scrollHeight;
  newMessagesElement.hidden = true;
}

function delightMascot(): void {
  if (delightTimer) clearTimeout(delightTimer);
  mascotElement.classList.remove("is-delighted");
  void mascotElement.offsetWidth;
  mascotElement.classList.add("is-delighted");
  delightTimer = setTimeout(() => mascotElement.classList.remove("is-delighted"), 700);
}

function initializeMotionControl(): void {
  let paused = false;
  try {
    const saved = localStorage.getItem(MOTION_STORAGE_KEY);
    paused =
      saved === "paused" ||
      (saved === null &&
        typeof window.matchMedia === "function" &&
        window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  } catch {
    paused = false;
  }
  setMotionPaused(paused);
  motionElement.addEventListener("click", () => {
    const nextPaused = document.documentElement.dataset.motion !== "paused";
    setMotionPaused(nextPaused);
    try {
      localStorage.setItem(MOTION_STORAGE_KEY, nextPaused ? "paused" : "playing");
    } catch {
      // Motion preference persistence is optional when storage is unavailable.
    }
  });
}

function setMotionPaused(paused: boolean): void {
  document.documentElement.dataset.motion = paused ? "paused" : "playing";
  motionElement.setAttribute("aria-pressed", String(paused));
  motionElement.setAttribute("aria-label", paused ? "Resume ambient motion" : "Pause ambient motion");
  if (motionLabelElement) motionLabelElement.textContent = paused ? "resume motion" : "pause motion";
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
