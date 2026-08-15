import { getAddress } from "ethers";
import { acolyteName } from "./acolyte-name";
import { XMTP_ENVIRONMENT, parseConfig } from "./config";
import { IdentityStorageError, loadOrCreateIdentity, type StoredIdentity } from "./identity";
import { initializeChatController, type ChatController } from "./chat/controller";
import { createXmtpWorkspace, ensureXmtpIdentityRegistration } from "./chat/xmtp-workspace";
import { parseOnboardingLink } from "./onboarding-links";
import "./style.css";

const INSTALLER_URL = "https://raw.githubusercontent.com/pierce403/cthuwu/main/install.sh";

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

let identity: StoredIdentity | undefined;
let controller: ChatController | undefined;

targetForm.addEventListener("submit", (event) => {
  event.preventDefault();
  try {
    const target = canonicalTarget(targetInput.value);
    location.hash = `t=${target}`;
    location.reload();
  } catch (error) {
    targetStatus.textContent = publicError(error);
  }
});

void bootstrap();

async function bootstrap(): Promise<void> {
  let target: string | undefined;
  let targetError: string | undefined;
  try {
    target = parseOnboardingLink(location.hash).tentacle;
  } catch (error) {
    targetError = publicError(error);
  }

  try {
    identity = loadOrCreateIdentity(XMTP_ENVIRONMENT);
    const base = parseConfig();
    nameElement.textContent = acolyteName(identity.address);
    addressElement.textContent = identity.address;
    targetStatus.textContent = "Registering this Acolyte address with XMTP…";
    inboxElement.textContent = await ensureXmtpIdentityRegistration(base, identity);
    prepareCommands(identity.address);

    if (!target) {
      targetStatus.textContent = targetError ??
        "Acolyte inbox ready. Enter one exact Tentacle wallet; no default or rotating target is used here.";
      targetInput.focus();
      return;
    }
    target = canonicalTarget(target);
    targetInput.value = target;
    targetStatus.textContent = `Direct route pinned to ${target}.`;
    chatElement.hidden = false;
    controller = initializeChatController(
      {
        ...base,
        botAddress: target,
        // Operator authority is deliberately independent of Branding ownership and assignment.
        brandingContract: undefined,
        tentacleAnchor: undefined,
        referrer: undefined,
        rotationAnchor: undefined,
      },
      identity,
      {
        brandingOffers: false,
        surface: "operator",
        createWorkspace: async (config, storedIdentity) => {
          const workspace = await createXmtpWorkspace(config, storedIdentity, {
            storage: sessionStorage,
          });
          if (workspace.inboxId !== inboxElement.textContent) {
            throw new Error("The operator workspace resolved a different XMTP inbox");
          }
          return workspace;
        },
      },
    );
    await controller.connect(false);
  } catch (error) {
    console.error(error);
    targetStatus.textContent = error instanceof IdentityStorageError
      ? error.message
      : publicError(error);
  }
}

function prepareCommands(address: string): void {
  const authorizeCommand = `./uwu.sh --data-dir /path/to/the-same-data-dir --xmtp-env production operator add ${address} --label WebAcolyte`;
  const launchCommand = `curl --proto '=https' --tlsv1.2 -fsSL ${INSTALLER_URL} | bash -s -- --operator ${address}`;
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
  if (!event.persisted) void controller?.close();
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted) void controller?.resume().catch(console.error);
});
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void controller?.resume().catch(console.error);
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
