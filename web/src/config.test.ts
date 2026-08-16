import { describe, expect, it } from "vitest";
import {
  CANONICAL_BRANDING_CONTRACT,
  CANONICAL_BRANDING_DEPLOYMENT_BLOCK,
  DEFAULT_BASE_RPC_ENDPOINT,
  INTRO_TENTACLE_ADDRESS,
  XMTP_ENVIRONMENT,
  parseConfig,
} from "./config";

describe("configuration", () => {
  it("always selects production XMTP", () => {
    expect(XMTP_ENVIRONMENT).toBe("production");
    expect(parseConfig().environment).toBe("production");
  });

  it("always selects the hard-coded intro Tentacle", () => {
    expect(INTRO_TENTACLE_ADDRESS).toBe("0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90");
    expect(parseConfig()).toEqual({
      environment: "production",
      botAddress: INTRO_TENTACLE_ADDRESS,
      baseRpcEndpoint: DEFAULT_BASE_RPC_ENDPOINT,
      brandingContract: CANONICAL_BRANDING_CONTRACT,
      assignmentRefreshMs: 600_000,
    });
  });

  it("defaults to the verified canonical Branding deployment", () => {
    expect(parseConfig().brandingContract).toBe(CANONICAL_BRANDING_CONTRACT);
    expect(CANONICAL_BRANDING_DEPLOYMENT_BLOCK).toBe(49_852_729n);
    expect(parseConfig({ VITE_CTHUWU_BRANDING_CONTRACT: "" }).brandingContract).toBe(
      CANONICAL_BRANDING_CONTRACT,
    );
  });

  it("accepts only the canonical Branding routing deployment", () => {
    expect(parseConfig({
      VITE_CTHUWU_BASE_RPC_ENDPOINT: "https://rpc.example/",
      VITE_CTHUWU_BRANDING_CONTRACT: CANONICAL_BRANDING_CONTRACT,
      VITE_CTHUWU_ASSIGNMENT_REFRESH_MS: "60000",
    })).toMatchObject({
      baseRpcEndpoint: "https://rpc.example/",
      brandingContract: CANONICAL_BRANDING_CONTRACT,
      assignmentRefreshMs: 60_000,
    });
    expect(() => parseConfig({
      VITE_CTHUWU_BRANDING_CONTRACT: "0x1111111111111111111111111111111111111111",
    })).toThrow(/canonical Base deployment/u);
    expect(() => parseConfig({ VITE_CTHUWU_BRANDING_CONTRACT: "0x0000000000000000000000000000000000000000" })).toThrow();
  });
});
