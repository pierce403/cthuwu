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
    expect(INTRO_TENTACLE_ADDRESS).toBe("0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db");
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

  it("accepts only explicit safe Branding routing configuration", () => {
    expect(parseConfig({
      VITE_CTHUWU_BASE_RPC_ENDPOINT: "https://rpc.example/",
      VITE_CTHUWU_BRANDING_CONTRACT: "0x1111111111111111111111111111111111111111",
      VITE_CTHUWU_ASSIGNMENT_REFRESH_MS: "60000",
    })).toMatchObject({
      baseRpcEndpoint: "https://rpc.example/",
      brandingContract: "0x1111111111111111111111111111111111111111",
      assignmentRefreshMs: 60_000,
    });
    expect(() => parseConfig({ VITE_CTHUWU_BRANDING_CONTRACT: "0x0000000000000000000000000000000000000000" })).toThrow();
  });
});
