import { describe, expect, it } from "vitest";
import { validateProductionConfig } from "./validate-production-config.mjs";

const configured = {
  VITE_CTHUWU_GRAPHQL_ENDPOINT:
    "https://gateway.thegraph.com/api/{api-key}/subgraphs/id/QmProduction",
  VITE_CTHUWU_GRAPH_API_KEY: "public_key-1234",
  VITE_CTHUWU_IPFS_GATEWAY: "https://ipfs.example/ipfs/",
  VITE_CTHUWU_ARWEAVE_GATEWAY: "https://ar.example/",
  VITE_CTHUWU_LEADERBOARD_FRESH_MS: "900000",
  VITE_CTHUWU_BRANDING_CONTRACT: "0xd8c36f13d79a505c7fbdc5f6467ea3cd75e896da",
  VITE_CTHUWU_ASSIGNMENT_REFRESH_MS: "600000",
};

describe("production static leaderboard configuration", () => {
  it("resolves an explicitly configured HTTPS Graph endpoint", () => {
    expect(validateProductionConfig(configured)).toBe(
      "https://gateway.thegraph.com/api/public_key-1234/subgraphs/id/QmProduction",
    );
    expect(
      validateProductionConfig({
        ...configured,
        VITE_CTHUWU_GRAPHQL_ENDPOINT:
          "https://gateway.example/{api-key}/subgraphs/{api-key}/query",
      }),
    ).toBe("https://gateway.example/public_key-1234/subgraphs/public_key-1234/query");
  });

  it("uses the pinned Agent0 Base subgraph when no endpoint override is supplied", () => {
    expect(validateProductionConfig({ VITE_CTHUWU_GRAPH_API_KEY: "public_key-1234" })).toBe(
      "https://gateway.thegraph.com/api/public_key-1234/subgraphs/id/43s9hQRurMGjuYnC1r2ZwS6xSQktbFyXMPMqGKUFJojb",
    );
  });

  it("uses the intentionally public checked-in key when no build variables are supplied", () => {
    expect(validateProductionConfig({})).toMatch(
      /^https:\/\/gateway\.thegraph\.com\/api\/[0-9a-f]{32}\/subgraphs\/id\/43s9h/u,
    );
  });

  it.each([undefined, ""])(
    "allows an absent or blank Branding contract (%s)",
    (brandingContract) => {
      expect(
        validateProductionConfig({
          ...configured,
          VITE_CTHUWU_BRANDING_CONTRACT: brandingContract,
        }),
      ).toContain("https://gateway.thegraph.com/");
    },
  );

  it.each([
    "0x0000000000000000000000000000000000000000",
    "0X1234567890abcdef1234567890abcdef12345678",
    "0x1234567890ABCDEF1234567890abcdef12345678",
    "0x1234567890abcdef1234567890abcdef1234567",
    "   ",
    " 0x1234567890abcdef1234567890abcdef12345678",
    "0x1234567890abcdef1234567890abcdef12345678 ",
  ])("rejects an invalid Branding contract: %s", (brandingContract) => {
    expect(() =>
      validateProductionConfig({
        ...configured,
        VITE_CTHUWU_BRANDING_CONTRACT: brandingContract,
      }),
    ).toThrow("canonical Base deployment");
  });

  it("rejects a well-formed alternate Branding deployment", () => {
    expect(() => validateProductionConfig({
      ...configured,
      VITE_CTHUWU_BRANDING_CONTRACT: "0x1234567890abcdef1234567890abcdef12345678",
    })).toThrow("canonical Base deployment");
  });

  it.each([undefined, ""])(
    "allows an absent or empty assignment refresh interval (%s)",
    (assignmentRefresh) => {
      expect(
        validateProductionConfig({
          ...configured,
          VITE_CTHUWU_ASSIGNMENT_REFRESH_MS: assignmentRefresh,
        }),
      ).toContain("https://gateway.thegraph.com/");
    },
  );

  it.each(["60000", "60001", "3600000"])(
    "accepts an in-range assignment refresh interval: %s",
    (assignmentRefresh) => {
      expect(
        validateProductionConfig({
          ...configured,
          VITE_CTHUWU_ASSIGNMENT_REFRESH_MS: assignmentRefresh,
        }),
      ).toContain("https://gateway.thegraph.com/");
    },
  );

  it.each([
    "59999",
    "3600001",
    " 60000",
    "60000 ",
    "60_000",
    "60000.0",
    "6e4",
    "+60000",
    "060000",
    "not-a-number",
  ])("rejects an invalid assignment refresh interval: %s", (assignmentRefresh) => {
    expect(() =>
      validateProductionConfig({
        ...configured,
        VITE_CTHUWU_ASSIGNMENT_REFRESH_MS: assignmentRefresh,
      }),
    ).toThrow("integer between 60000 and 3600000");
  });

  it.each([
    [{ ...configured, VITE_CTHUWU_GRAPH_API_KEY: "REPLACE_ME" }, "must resolve"],
    [
      { ...configured, VITE_CTHUWU_GRAPHQL_ENDPOINT: "https://gateway.example/{deployment}" },
      "unresolved placeholder",
    ],
    [{ ...configured, VITE_CTHUWU_GRAPHQL_ENDPOINT: "http://example.test/graphql" }, "HTTPS"],
    [{ ...configured, VITE_CTHUWU_GRAPHQL_ENDPOINT: "https://user:pass@example.test" }, "credential-free"],
    [{ ...configured, VITE_CTHUWU_LEADERBOARD_FRESH_MS: "1" }, "between"],
  ])("rejects an unsafe or incomplete deployment environment", (environment, message) => {
    expect(() => validateProductionConfig(environment)).toThrow(message);
  });
});
