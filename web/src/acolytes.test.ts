import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const catalog = vi.hoisted(() => ({
  fetch: vi.fn(async () => ({
    chainId: 8453,
    contractAddress: "0xd8c36f13d79a505c7fbdc5f6467ea3cd75e896da",
    sourceBlockNumber: "49853711",
    sourceBlockHash: `0x${"ab".repeat(32)}`,
    items: [
      {
        tokenId: "17",
        acolyte: "0x0000000000000000000000000000000000000011",
        owner: "0x0000000000000000000000000000000000000022",
        controllerAgentId: "7",
        referrer: "0x0000000000000000000000000000000000000033",
        declaredPrice: "1000000000000000000",
        paidThrough: "1787000000",
        pendingDeclaredPrice: "0",
        pendingPriceValidAfter: "0",
        status: "Active",
        avatarUri: "javascript:alert(document.cookie)",
        traits: [{ traitType: "<img src=x>", value: "<script>boom()</script>" }],
        mintBlockNumber: "49852729",
        mintTransactionHash: `0x${"cd".repeat(32)}`,
      },
    ],
  })),
}));

vi.mock("./acolyte-data", () => ({ fetchAcolyteCatalog: catalog.fetch }));

const html = readFileSync(resolve(process.cwd(), "acolytes/index.html"), "utf8");

describe("Acolyte catalog page", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    const parsed = new DOMParser().parseFromString(html, "text/html");
    document.head.innerHTML = parsed.head.innerHTML;
    document.body.innerHTML = parsed.body.innerHTML;
  });

  it("renders owner metadata as inert text without loading or linking unsafe avatars", async () => {
    await import("./acolytes");
    await vi.waitFor(() => expect(document.querySelector("#acolyte-state")?.textContent).toBe("CURRENT"));
    const card = document.querySelector(".acolyte-card");
    expect(card?.textContent).toContain("javascript:alert(document.cookie)");
    expect(card?.textContent).toContain("<img src=x>");
    expect(card?.textContent).toContain("<script>boom()</script>");
    expect(card?.querySelector("script")).toBeNull();
    expect(card?.querySelector('a[href^="javascript:"]')).toBeNull();
    expect(card?.querySelector("img")).toBeNull();
  });
});
