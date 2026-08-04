import { describe, expect, it } from "vitest";
import { INTRO_TENTACLE_ADDRESS, XMTP_ENVIRONMENT, parseConfig } from "./config";

describe("configuration", () => {
  it("always selects production XMTP", () => {
    expect(XMTP_ENVIRONMENT).toBe("production");
    expect(parseConfig().environment).toBe("production");
  });

  it("always selects the hard-coded intro Tentacle", () => {
    expect(INTRO_TENTACLE_ADDRESS).toBe("0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db");
    expect(parseConfig()).toEqual({
      environment: "production",
      botAddress: INTRO_TENTACLE_ADDRESS,
    });
  });
});
