import { describe, expect, it, vi } from "vitest";
import { recoverRegisteredClient } from "./xmtp-transport";

describe("Browser SDK installation recovery", () => {
  it("reuses an existing registered installation without registering again", async () => {
    const client = {
      isRegistered: vi.fn().mockResolvedValue(true),
      register: vi.fn(),
      close: vi.fn(),
    };

    await expect(recoverRegisteredClient(async () => client)).resolves.toBe(client);
    expect(client.isRegistered).toHaveBeenCalledOnce();
    expect(client.register).not.toHaveBeenCalled();
    expect(client.close).not.toHaveBeenCalled();
  });

  it("registers only a new installation", async () => {
    const client = {
      isRegistered: vi.fn().mockResolvedValue(false),
      register: vi.fn().mockResolvedValue(undefined),
      close: vi.fn(),
    };

    await recoverRegisteredClient(async () => client);
    expect(client.register).toHaveBeenCalledOnce();
    expect(client.close).not.toHaveBeenCalled();
  });

  it("closes a failed registration attempt so reconnect can recover cleanly", async () => {
    const client = {
      isRegistered: vi.fn().mockResolvedValue(false),
      register: vi.fn().mockRejectedValue(new Error("installation limit")),
      close: vi.fn(),
    };

    await expect(recoverRegisteredClient(async () => client)).rejects.toThrow(
      "installation limit",
    );
    expect(client.close).toHaveBeenCalledOnce();
  });
});
