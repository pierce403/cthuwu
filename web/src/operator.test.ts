import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, expect, it, vi } from "vitest";
const transport = vi.hoisted(() => ({ open: vi.fn(), add: vi.fn(), resume: vi.fn(async () => {}), close: vi.fn(async () => {}), changed: undefined as (() => void) | undefined, threads: new Map(), selected: undefined as string | undefined, connected: true, client: { inboxId: "a".repeat(64) }, ordered: vi.fn(() => []), registerReferral: vi.fn(async () => {}) }));
vi.mock("./chat/xmtp-workspace", () => ({ openRegisteredClient: transport.open }));
vi.mock("./operator-inbox", () => ({ OperatorInbox: class {
  constructor(_client: unknown, _storage: unknown, changed: () => void) { transport.changed = changed; return transport; }
} }));
const html = readFileSync(resolve(process.cwd(), "operator/index.html"), "utf8");
beforeEach(() => {
  vi.resetModules(); vi.clearAllMocks(); localStorage.clear(); location.hash = "";
  const parsed = new DOMParser().parseFromString(html, "text/html"); document.body.innerHTML = parsed.body.innerHTML;
  transport.threads.clear(); transport.selected = undefined;
  transport.open.mockResolvedValue({ client: transport.client, releaseDatabaseLease: vi.fn() });
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn(async () => {}) } });
});
it("opens one shared inbox even before a Tentacle is selected", async () => {
  await import("./operator"); await vi.waitFor(() => expect(transport.resume).toHaveBeenCalled());
  expect(transport.open).toHaveBeenCalledTimes(1); expect(transport.add).not.toHaveBeenCalled();
  expect(document.querySelector("#operator-inbox")?.textContent).toBe(transport.client.inboxId);
  expect(document.querySelector<HTMLButtonElement>("#operator-copy-launch")?.disabled).toBe(false);
});
it("keeps a visible copyable referral link bound to the selected Tentacle", async () => {
  await import("./operator"); await vi.waitFor(() => expect(transport.resume).toHaveBeenCalled());
  const wallet = "0x" + "2".repeat(40);
  transport.selected = "two"; transport.threads.set("two", { id: "two", label: "Nyx", wallet, saved: true, unread: 0, draft: "", messages: new Map() }); transport.changed?.();
  const link = document.querySelector<HTMLInputElement>("#operator-referral-link")!.value;
  expect(link).toContain(`#t=${wallet}&r=0x`);
  expect(document.querySelector<HTMLElement>("#operator-referral")?.hidden).toBe(false);
  document.querySelector<HTMLButtonElement>("#operator-copy-referral")?.click();
  await vi.waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(link));
  expect(transport.registerReferral).toHaveBeenCalledWith("two");
});
