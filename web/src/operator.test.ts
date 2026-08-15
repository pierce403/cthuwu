import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { isLocalIdentity, loadOrCreateIdentity } from "./identity";

const inboxId = "a".repeat(64);
const chat = vi.hoisted(() => ({
  connect: vi.fn(async () => undefined),
  resume: vi.fn(async () => undefined),
  close: vi.fn(async () => undefined),
  initialize: vi.fn(),
}));
const xmtp = vi.hoisted(() => ({
  ensure: vi.fn(),
  create: vi.fn(),
}));

vi.mock("./chat/controller", () => ({
  initializeChatController: chat.initialize.mockReturnValue(chat),
}));
vi.mock("./chat/xmtp-workspace", () => ({
  ensureXmtpIdentityRegistration: xmtp.ensure,
  createXmtpWorkspace: xmtp.create,
}));

const html = readFileSync(resolve(process.cwd(), "operator/index.html"), "utf8");
const target = "0x2222222222222222222222222222222222222222";

function mount(): void {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  document.head.innerHTML = parsed.head.innerHTML;
  document.body.innerHTML = parsed.body.innerHTML;
}

describe("operator console", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    chat.initialize.mockReturnValue(chat);
    xmtp.ensure.mockResolvedValue(inboxId);
    xmtp.create.mockResolvedValue({ inboxId });
    localStorage.clear();
    sessionStorage.clear();
    location.hash = "";
    mount();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => undefined) },
    });
  });

  it("registers and reuses the canonical Acolyte identity without selecting a target", async () => {
    const identity = loadOrCreateIdentity("production");
    if (!isLocalIdentity(identity)) throw new Error("expected local identity");
    await import("./operator");
    await vi.waitFor(() => expect(xmtp.ensure).toHaveBeenCalled());

    expect(chat.initialize).not.toHaveBeenCalled();
    expect(xmtp.ensure).toHaveBeenCalledWith(
      expect.objectContaining({ environment: "production" }),
      identity,
    );
    expect(document.querySelector("#operator-target-status")?.textContent).toContain(
      "no default or rotating target",
    );
    expect(document.querySelector("#operator-address")?.textContent).toBe(identity.address);
    expect(document.querySelector("#operator-inbox")?.textContent).toBe(inboxId);
    expect(document.querySelector("#operator-name")?.textContent).toMatch(/^[A-Z][A-Za-z-]+ of [A-Z]/u);
    expect(document.querySelector<HTMLElement>("#chat")?.hidden).toBe(true);

    const authorize = document.querySelector("#operator-authorize-command")?.textContent ?? "";
    const launch = document.querySelector("#operator-launch-command")?.textContent ?? "";
    expect(authorize).toContain(`operator add ${identity.address} --label WebAcolyte`);
    expect(authorize).toContain("--data-dir /path/to/the-same-data-dir");
    expect(launch).toContain("curl --proto '=https' --tlsv1.2 -fsSL");
    expect(launch).toContain(`--operator ${identity.address}`);
    expect(`${authorize}\n${launch}\n${document.body.textContent}`).not.toContain(identity.walletPrivateKey);
    expect(authorize).not.toContain("private-key");
    expect(launch).not.toContain("private-key");

    document.querySelector<HTMLButtonElement>("#operator-copy-launch")?.click();
    await vi.waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(launch));
  });

  it("keeps both commands disabled until the Acolyte inbox is registered", async () => {
    let resolveInbox!: (value: string) => void;
    xmtp.ensure.mockReturnValue(new Promise((resolve) => { resolveInbox = resolve; }));
    await import("./operator");
    expect(document.querySelector<HTMLButtonElement>("#operator-copy-authorize")?.disabled).toBe(true);
    expect(document.querySelector<HTMLButtonElement>("#operator-copy-launch")?.disabled).toBe(true);

    resolveInbox(inboxId);
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>("#operator-copy-authorize")?.disabled).toBe(false);
      expect(document.querySelector<HTMLButtonElement>("#operator-copy-launch")?.disabled).toBe(false);
    });
  });

  it("opens only the explicit direct route, independent of Branding assignment", async () => {
    const identity = loadOrCreateIdentity("production");
    location.hash = `t=${target}`;
    await import("./operator");
    await vi.waitFor(() => expect(chat.connect).toHaveBeenCalledWith(false));
    expect(chat.initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        botAddress: target,
        brandingContract: undefined,
        tentacleAnchor: undefined,
        referrer: undefined,
        rotationAnchor: undefined,
      }),
      identity,
      expect.objectContaining({ brandingOffers: false, surface: "operator" }),
    );
    expect(document.querySelector<HTMLElement>("#chat")?.hidden).toBe(false);

    const dependencies = chat.initialize.mock.calls[0]?.[2] as {
      createWorkspace(config: unknown, storedIdentity: unknown): Promise<unknown>;
    };
    const directConfig = chat.initialize.mock.calls[0]?.[0];
    await dependencies.createWorkspace(directConfig, identity);
    expect(xmtp.create).toHaveBeenCalledWith(directConfig, identity, {
      storage: sessionStorage,
    });
  });
});
