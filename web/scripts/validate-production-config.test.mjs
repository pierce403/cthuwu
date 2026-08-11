import { describe, expect, it } from "vitest";
import { validateProductionConfig } from "./validate-production-config.mjs";

const configured = {
  VITE_CTHUWU_GRAPHQL_ENDPOINT:
    "https://gateway.thegraph.com/api/{api-key}/subgraphs/id/QmProduction",
  VITE_CTHUWU_GRAPH_API_KEY: "public_key-1234",
  VITE_CTHUWU_IPFS_GATEWAY: "https://ipfs.example/ipfs/",
  VITE_CTHUWU_ARWEAVE_GATEWAY: "https://ar.example/",
  VITE_CTHUWU_LEADERBOARD_FRESH_MS: "900000",
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

  it.each([
    [{ ...configured, VITE_CTHUWU_GRAPHQL_ENDPOINT: "" }, "must identify"],
    [{ ...configured, VITE_CTHUWU_GRAPH_API_KEY: "" }, "must resolve"],
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
