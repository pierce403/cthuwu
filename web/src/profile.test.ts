import { describe, expect, it } from "vitest";
import { parseDataRegistration } from "./profile";

function dataUri(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  return `data:application/json;base64,${btoa(String.fromCharCode(...bytes))}`;
}

describe("hostile public profile handling", () => {
  it("parses only bounded registration-v1 data documents", () => {
    const profile = parseDataRegistration(
      dataUri({
        type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
        name: "The Indexed One",
        description: "A public Tentacle",
        image: "ipfs://bafyavatar",
        active: true,
        services: [
          { name: "CTHUWU-XMTP", endpoint: `xmtp://${"a".repeat(64)}` },
          { name: "CTHUWU", endpoint: "https://cthuwu.app/manifest.json" },
        ],
      }),
      "42",
    );
    expect(profile).toMatchObject({
      name: "The Indexed One",
      active: true,
      image: "ipfs://bafyavatar",
      xmtpEndpoint: `xmtp://${"a".repeat(64)}`,
    });
    expect(
      parseDataRegistration(
        dataUri({
          type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
          name: "Legacy service label",
          active: true,
          services: [{ name: "XMTP", endpoint: `xmtp://${"b".repeat(64)}` }],
        }),
        "44",
      )?.xmtpEndpoint,
    ).toBe(`xmtp://${"b".repeat(64)}`);
    expect(
      parseDataRegistration(
        dataUri({
          type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
          name: "Céphalopode 🐙",
          active: true,
        }),
        "43",
      )?.name,
    ).toBe("Céphalopode 🐙");
  });

  it("rejects wrong schemas, control characters, unsafe endpoints, and oversized data", () => {
    expect(parseDataRegistration(dataUri({ type: "agent", name: "wrong" }), "1")).toBeUndefined();
    const profile = parseDataRegistration(
      dataUri({
        type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
        name: "bad\u0000name",
        description: "ok",
        image: "javascript:alert(1)",
        active: true,
        services: [
          { name: "XMTP", endpoint: "javascript:alert(1)" },
          { name: "CTHUWU-XMTP", endpoint: `xmtp://${"A".repeat(64)}` },
          { name: "CTHUWU-XMTP", endpoint: `xmtp:inbox:${"a".repeat(64)}` },
        ],
      }),
      "1",
    );
    expect(profile?.name).toBe("Tentacle #1");
    expect(profile?.image).toBeUndefined();
    expect(profile?.xmtpEndpoint).toBeUndefined();
    expect(
      parseDataRegistration(
        dataUri({
          type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
          name: "safe\u202egnp.exe",
          active: true,
        }),
        "2",
      )?.name,
    ).toBe("Tentacle #2");
    expect(parseDataRegistration(`data:application/json;base64,${"A".repeat(30_000)}`, "1")).toBeUndefined();
    expect(
      parseDataRegistration(
        dataUri({
          type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
          services: Array.from({ length: 17 }, () => ({ name: "CTHUWU", endpoint: "https://example.test" })),
        }),
        "1",
      ),
    ).toBeUndefined();
    let nested: unknown = "leaf";
    for (let depth = 0; depth < 10; depth += 1) nested = { nested };
    expect(
      parseDataRegistration(
        dataUri({
          type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
          nested,
        }),
        "1",
      ),
    ).toBeUndefined();
  });
});
