import { describe, expect, it, vi } from "vitest";
import { resolveOperatorIdentity } from "./operator-identity.js";

const INBOX = "ab".repeat(32);

describe("resolveOperatorIdentity", () => {
  it("looks up a checksummed Ethereum address on the selected XMTP network", async () => {
    const resolveInbox = vi.fn(async () => INBOX.toUpperCase());
    const result = await resolveOperatorIdentity(
      "0x4200000000000000000000000000000000000006",
      "production",
      { resolveInbox },
    );

    expect(result).toEqual({
      address: "0x4200000000000000000000000000000000000006",
      inboxId: INBOX,
    });
    expect(resolveInbox).toHaveBeenCalledWith(result.address, "production");
  });

  it("normalizes ENS, resolves it on Ethereum mainnet, then looks up XMTP", async () => {
    const resolveEns = vi.fn(async () =>
      Promise.resolve("0x4200000000000000000000000000000000000006" as const),
    );
    const resolveInbox = vi.fn(async () => Promise.resolve(INBOX));

    const result = await resolveOperatorIdentity("DEAN.ETH", "production", {
      resolveEns,
      resolveInbox,
    });

    expect(resolveEns).toHaveBeenCalledWith("dean.eth");
    expect(result.inboxId).toBe(INBOX);
  });

  it("rejects non-ENS names and addresses without a production inbox", async () => {
    await expect(
      resolveOperatorIdentity("dean.example", "production"),
    ).rejects.toThrow("full 0x Ethereum address or .eth ENS name");
    await expect(
      resolveOperatorIdentity(
        "0x4200000000000000000000000000000000000006",
        "production",
        { resolveInbox: async () => null },
      ),
    ).rejects.toThrow("has no inbox on XMTP production");
  });
});
