import { XMTP_ENVIRONMENT, parseConfig } from "./config";
import {
  IdentityStorageError,
  loadOrCreateIdentity,
  persistIdentity,
  resetIdentity,
  type StoredIdentity,
} from "./identity";
import { decryptIdentityBackup, encryptIdentityBackup } from "./identity-backup";
import { initializePwaInstallPrompt, type PwaInstallController } from "./pwa";
import { initializeLeaderboard, type LeaderboardController } from "./leaderboard";
import { initializeChatController, type ChatController } from "./chat/controller";
import "./style.css";

const MOTION_STORAGE_KEY = "cthuwu.ui.motion.v1";
const environment = XMTP_ENVIRONMENT;

const mascotElement = requireElement<HTMLElement>("mascot-stage");
const motionElement = requireElement<HTMLButtonElement>("motion-toggle");
const motionLabelElement = motionElement.querySelector("span");
const settingsElement = requireElement<HTMLButtonElement>("settings");
const dialogElement = requireElement<HTMLDialogElement>("identity-dialog");
const dialogCloseElement = requireElement<HTMLButtonElement>("identity-close");
const addressElement = requireElement<HTMLElement>("identity-address");
const environmentElement = requireElement<HTMLElement>("identity-environment");
const passphraseElement = requireElement<HTMLInputElement>("backup-passphrase");
const exportElement = requireElement<HTMLButtonElement>("export-identity");
const importElement = requireElement<HTMLInputElement>("import-identity");
const resetElement = requireElement<HTMLButtonElement>("reset-identity");
const settingsStatusElement = requireElement<HTMLParagraphElement>("settings-status");
const installPromptElement = requireElement<HTMLElement>("install-prompt");
const installTitleElement = requireElement<HTMLElement>("install-title");
const installCopyElement = requireElement<HTMLElement>("install-copy");
const installActionElement = requireElement<HTMLButtonElement>("install-action");
const installDismissElement = requireElement<HTMLButtonElement>("install-dismiss");
const installMenuElement = requireElement<HTMLButtonElement>("install-app");
const updatePromptElement = requireElement<HTMLElement>("update-prompt");
const updateActionElement = requireElement<HTMLButtonElement>("update-action");
const updateDismissElement = requireElement<HTMLButtonElement>("update-dismiss");

let identity: StoredIdentity | undefined;
let chatController: ChatController | undefined;
let leaderboardController: LeaderboardController | undefined;
let pwaController: PwaInstallController | undefined;

bootstrap();

function bootstrap(): void {
  environmentElement.textContent = environment;
  wireIdentityControls();
  initializeMotionControl();
  try {
    // Preserve the backup-critical boundary: create/recover the browser identity before config or I/O.
    identity = loadOrCreateIdentity(environment);
    addressElement.textContent = identity.address;
    chatController = initializeChatController(parseConfig(), identity);
  } catch (error) {
    fatalIdentity(error);
  }
  initializePublicFeatures();
  if (chatController) void chatController.connect(false);
}

function initializePublicFeatures(): void {
  try {
    leaderboardController = initializeLeaderboard({
      root: requireElement<HTMLElement>("tentacles"),
      status: requireElement<HTMLElement>("leaderboard-state"),
      source: requireElement<HTMLElement>("leaderboard-source"),
      summary: requireElement<HTMLElement>("leaderboard-summary"),
      refresh: requireElement<HTMLButtonElement>("leaderboard-refresh"),
      search: requireElement<HTMLInputElement>("leaderboard-search"),
      funding: requireElement<HTMLSelectElement>("leaderboard-funding"),
      verification: requireElement<HTMLSelectElement>("leaderboard-verification"),
      protocol: requireElement<HTMLSelectElement>("leaderboard-protocol"),
      shared: requireElement<HTMLSelectElement>("leaderboard-shared"),
      sort: requireElement<HTMLSelectElement>("leaderboard-sort"),
      ranked: requireElement<HTMLElement>("leaderboard-list"),
      suspendedSection: requireElement<HTMLElement>("leaderboard-suspended-section"),
      suspended: requireElement<HTMLElement>("leaderboard-suspended"),
      empty: requireElement<HTMLElement>("leaderboard-empty"),
    });
  } catch (error) {
    console.error("The public leaderboard configuration is invalid", error);
    requireElement<HTMLElement>("leaderboard-state").textContent = "UNAVAILABLE";
    requireElement<HTMLElement>("leaderboard-source").textContent =
      "Leaderboard configuration unavailable";
  }
  pwaController = initializePwaInstallPrompt(
    {
      card: installPromptElement,
      title: installTitleElement,
      copy: installCopyElement,
      action: installActionElement,
      dismiss: installDismissElement,
      menuAction: installMenuElement,
    },
    {
      enabled: import.meta.env.PROD,
      onBackupRequested: () => dialogElement.showModal(),
      onMenuRequested: () => {
        if (dialogElement.open) dialogElement.close();
        return settingsElement;
      },
      updateElements: {
        card: updatePromptElement,
        action: updateActionElement,
        dismiss: updateDismissElement,
      },
    },
  );
}

function wireIdentityControls(): void {
  dialogElement.querySelector("form")?.addEventListener("submit", (event) => event.preventDefault());
  settingsElement.addEventListener("click", () => dialogElement.showModal());
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
    const restored = await decryptIdentityBackup(await file.text(), passphraseElement.value, environment);
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
  await chatController?.close();
  resetIdentity(environment);
  location.reload();
}

window.addEventListener("pagehide", (event) => {
  if (event.persisted) return;
  leaderboardController?.dispose();
  pwaController?.dispose();
  void chatController?.close();
});

window.addEventListener("pageshow", (event) => {
  if (event.persisted) void chatController?.resume().catch(console.error);
});

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void chatController?.resume().catch(console.error);
});

function fatalIdentity(error: unknown): void {
  console.error(error);
  requireElement<HTMLElement>("chat").dataset.state = "fatal-error";
  mascotElement.dataset.mood = "worried";
  requireElement<HTMLElement>("status").textContent = error instanceof IdentityStorageError
    ? error.message
    : "The browser identity could not be created or loaded";
  dialogElement.showModal();
}

function initializeMotionControl(): void {
  let paused = false;
  try {
    const saved = localStorage.getItem(MOTION_STORAGE_KEY);
    paused = saved === "paused" || (saved === null && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
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
      // Motion preference persistence is optional.
    }
  });
}

function setMotionPaused(paused: boolean): void {
  document.documentElement.dataset.motion = paused ? "paused" : "playing";
  motionElement.setAttribute("aria-pressed", String(paused));
  motionElement.setAttribute("aria-label", paused ? "Resume ambient motion" : "Pause ambient motion");
  if (motionLabelElement) motionLabelElement.textContent = paused ? "resume motion" : "pause motion";
}

function setSettingsStatus(message: string): void {
  settingsStatusElement.textContent = message;
}

function publicError(error: unknown): string {
  return error instanceof Error ? error.message : "The identity operation failed";
}

function requireElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing #${id}`);
  return element as T;
}
