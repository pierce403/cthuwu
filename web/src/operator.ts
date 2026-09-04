import { getAddress } from "ethers";
import { acolyteName } from "./acolyte-name";
import { XMTP_ENVIRONMENT, parseConfig } from "./config";
import { IdentityStorageError, loadOrCreateIdentity, type StoredIdentity } from "./identity";
import { OperatorInbox } from "./operator-inbox";
import { openRegisteredClient } from "./chat/xmtp-workspace";
import { parseOnboardingLink, recruitmentUrl } from "./onboarding-links";
import "./style.css";

const INSTALLER_URL = "https://cthuwu.app/install.sh";

const targetForm = required<HTMLFormElement>("operator-target-form");
const targetInput = required<HTMLInputElement>("operator-target");
const targetStatus = required<HTMLElement>("operator-target-status");
const nameElement = required<HTMLElement>("operator-name");
const addressElement = required<HTMLElement>("operator-address");
const inboxElement = required<HTMLElement>("operator-inbox");
const authorizeCommandElement = required<HTMLElement>("operator-authorize-command");
const authorizeCopyElement = required<HTMLButtonElement>("operator-copy-authorize");
const authorizeCopyStatus = required<HTMLElement>("operator-authorize-copy-status");
const launchCommandElement = required<HTMLElement>("operator-launch-command");
const launchCopyElement = required<HTMLButtonElement>("operator-copy-launch");
const launchCopyStatus = required<HTMLElement>("operator-launch-copy-status");
const chatElement = required<HTMLElement>("chat");
const routeElement = requiredSelector<HTMLElement>(".operator-route");

let identity: StoredIdentity | undefined;
let inbox: OperatorInbox | undefined;
let renderedThread: string | undefined;
const sending = new Set<string>();
const sendErrors = new Map<string, string>();

targetForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void openTarget(targetInput.value);
});

async function openTarget(value: string): Promise<void> {
  try {
    const target = canonicalTarget(value);
    if (!inbox) throw new Error("Your XMTP inbox is still connecting. Try again when it is ready.");
    await inbox.add(target);
    history.replaceState(null, "", `#t=${target}`);
    targetStatus.textContent = "Conversation open. The Tentacle checks your operator authority.";
  } catch (error) { targetStatus.textContent = publicError(error); }
}

required<HTMLFormElement>("operator-label-form").addEventListener("submit", event => {
  event.preventDefault();
  if (inbox?.selected) inbox.label(inbox.selected, required<HTMLInputElement>("operator-label").value);
});
required<HTMLTextAreaElement>("message").addEventListener("input", event => {
  if (inbox?.selected) inbox.draft(inbox.selected, (event.target as HTMLTextAreaElement).value);
});
required<HTMLFormElement>("composer").addEventListener("submit", event => {
  event.preventDefault();
  const id = inbox?.selected;
  const text = required<HTMLTextAreaElement>("message").value;
  if (!id || !inbox) return;
  const button = required<HTMLButtonElement>("send");
  button.disabled = true; sending.add(id);
  void inbox.send(id, text).then(() => {
    if (inbox?.selected === id) required<HTMLTextAreaElement>("message").value = inbox.threads.get(id)?.draft ?? "";
    sendErrors.delete(id);
  }).catch(error => {
    sendErrors.set(id, `${publicError(error)} Check the conversation before retrying; delivery may have completed.`);
  }).finally(() => { sending.delete(id); render(); });
});
required<HTMLButtonElement>("load-earlier").addEventListener("click", () => {
  if (inbox?.selected) void inbox.history(inbox.selected, true).catch(error => { targetStatus.textContent = publicError(error); });
});
required<HTMLButtonElement>("connect").addEventListener("click", () => void inbox?.resume().catch(error => { targetStatus.textContent = publicError(error); }));
async function shareReferral(share: boolean): Promise<void> {
  const id = inbox?.selected;
  if (!inbox || !id || !identity) return;
  const thread = inbox.threads.get(id)!;
  const link = recruitmentUrl(location.origin, thread.wallet, identity.address);
  const status = required("operator-referral-status");
  status.textContent = "Checking referral activation with this Tentacle…";
  try {
    await inbox.registerReferral(id);
    if (share && navigator.share) await navigator.share({ title: "Meet my Cthuwu Tentacle", url: link });
    else if (navigator.clipboard) await navigator.clipboard.writeText(link);
    if (inbox.selected === id) status.textContent = navigator.clipboard || share ? "Referral link ready to share." : "Select and copy the displayed link.";
  } catch (error) { if (inbox.selected === id) status.textContent = publicError(error); }
}
required<HTMLButtonElement>("operator-copy-referral").addEventListener("click", () => void shareReferral(false));
required<HTMLButtonElement>("operator-share-referral").addEventListener("click", () => void shareReferral(true));

void bootstrap();

async function bootstrap(): Promise<void> {
  try {
    identity = loadOrCreateIdentity(XMTP_ENVIRONMENT);
    nameElement.textContent = acolyteName(identity.address);
    addressElement.textContent = identity.address;
    targetStatus.textContent = "Connecting your operator inbox…";
    const base = parseConfig();
    const { client, releaseDatabaseLease } = await openRegisteredClient(base, identity);
    inboxElement.textContent = client.inboxId!;
    prepareCommands(identity.address);
    inbox = new OperatorInbox(client, localStorage, render, releaseDatabaseLease, base.environment);
    await inbox.resume();
    const target = parseOnboardingLink(location.hash).tentacle;
    if (target) { targetInput.value = target; await openTarget(target); }
    else targetStatus.textContent = "Inbox ready. Add a Tentacle or select an incoming conversation.";
    render();
  } catch (error) {
    targetStatus.textContent = error instanceof IdentityStorageError ? error.message : publicError(error);
  }
}

function render(): void {
  if (!inbox) return;
  const list = required("operator-threads");
  const threads = [...inbox.threads.values()].sort((a, b) => Number(b.saved) - Number(a.saved) || b.unread - a.unread || a.label.localeCompare(b.label));
  const buttons = threads.map(thread => {
    const button = document.createElement("button");
    button.type = "button"; button.className = "operator-thread";
    button.setAttribute("aria-pressed", String(thread.id === inbox?.selected));
    const latest = inbox!.ordered(thread).at(-1);
    button.textContent = `${thread.label}${thread.unread ? ` · ${thread.unread} unread` : ""}${thread.saved ? "" : " · new contact"}${latest ? `\n${latest.sentAt.toLocaleString()}` : ""}`;
    button.addEventListener("click", () => inbox?.select(thread.id));
    return button;
  });
  list.replaceChildren(...buttons);
  const thread = inbox.selected ? inbox.threads.get(inbox.selected) : undefined;
  if (!thread) return;
  const changed = renderedThread !== thread.id;
  renderedThread = thread.id;
  chatElement.hidden = false; routeElement.classList.add("is-console-active");
  chatElement.dataset.state = inbox.connected ? "connected" : "retryable-error";
  required("operator-label-form").hidden = false;
  if (changed) {
    required("operator-referral-status").textContent = "";
    required<HTMLInputElement>("operator-label").value = thread.label;
    required<HTMLTextAreaElement>("message").value = thread.draft;
  }
  required("chat-name").textContent = thread.label;
  required("status").textContent = inbox.connected ? "Connected · authority checked by the Tentacle" : "Reconnecting…";
  required("composer-error").hidden = !sendErrors.has(thread.id);
  required("composer-error").textContent = sendErrors.get(thread.id) ?? "";
  required<HTMLButtonElement>("connect").hidden = inbox.connected;
  required<HTMLTextAreaElement>("message").disabled = !inbox.connected;
  required<HTMLButtonElement>("send").disabled = !inbox.connected || sending.has(thread.id);
  required("retention-notice").textContent = "Messages use 14-day disappearing history. Other clients may retain copies.";
  required("operator-referral").hidden = false;
  required<HTMLInputElement>("operator-referral-link").value = recruitmentUrl(location.origin, thread.wallet, identity!.address);
  required<HTMLButtonElement>("operator-share-referral").hidden = !navigator.share;
  const messages = required("messages");
  const atBottom = messages.scrollHeight - messages.scrollTop - messages.clientHeight < 80;
  messages.replaceChildren(...inbox.ordered(thread).map(message => {
    const article = document.createElement("article");
    article.className = message.senderInboxId === inbox!.client.inboxId ? "message user-message" : "message bot-message";
    const label = document.createElement("small");
    label.textContent = `${message.senderInboxId === inbox!.client.inboxId ? "You" : thread.label} · ${message.sentAt.toLocaleTimeString()}`;
    const body = document.createElement("div"); body.className = "operator-message-body"; body.textContent = String(message.content);
    article.append(label, body); return article;
  }));
  messages.classList.toggle("is-empty", thread.messages.size === 0);
  messages.setAttribute("aria-busy", "false");
  required("load-earlier").hidden = thread.messages.size < 80;
  if (changed || atBottom) messages.scrollTop = messages.scrollHeight;
}

function prepareCommands(address: string): void {
  const authorizeCommand = `./uwu.sh --data-dir /path/to/the-same-data-dir operator add ${address} --label WebAcolyte`;
  const launchCommand = `curl -fsSL ${INSTALLER_URL} | bash -s -- --operator ${address}`;
  authorizeCommandElement.textContent = authorizeCommand;
  launchCommandElement.textContent = launchCommand;
  bindCopy(authorizeCopyElement, authorizeCopyStatus, authorizeCommand, "Existing-node command copied.");
  bindCopy(launchCopyElement, launchCopyStatus, launchCommand, "New-Tentacle command copied.");
}

function bindCopy(
  button: HTMLButtonElement,
  status: HTMLElement,
  command: string,
  success: string,
): void {
  button.disabled = false;
  button.addEventListener("click", () => {
    if (!navigator.clipboard) {
      status.textContent = "Clipboard access is unavailable; copy the displayed command manually.";
      return;
    }
    void navigator.clipboard.writeText(command).then(() => {
      status.textContent = success;
    }).catch(() => {
      status.textContent = "Could not copy; copy the displayed command manually.";
    });
  });
}

window.addEventListener("pagehide", (event) => {
  if (!event.persisted) void inbox?.close();
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted) void inbox?.resume().catch(console.error);
});
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void inbox?.resume().catch(console.error);
});

function canonicalTarget(value: string): string {
  const target = getAddress(value.trim()).toLowerCase();
  if (target === "0x0000000000000000000000000000000000000000") {
    throw new Error("Tentacle wallet must be nonzero");
  }
  return target;
}

function publicError(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "The operator route could not open.";
}

function required<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing operator element #${id}`);
  return element as T;
}

function requiredSelector<T extends HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing operator element ${selector}`);
  return element;
}
