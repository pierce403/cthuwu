import { describe, expect, it } from "vitest";
import { parseLeaderboardConfig } from "./leaderboard-config";

describe("leaderboard configuration", () => {
  it("substitutes a public Graph key while keeping Base-only chain constants elsewhere", () => {
    expect(
      parseLeaderboardConfig({
        VITE_CTHUWU_GRAPHQL_ENDPOINT:
          "https://gateway.thegraph.com/api/{api-key}/subgraphs/id/example",
        VITE_CTHUWU_GRAPH_API_KEY: "public-key",
      }).graphEndpoint,
    ).toBe("https://gateway.thegraph.com/api/public-key/subgraphs/id/example");
    expect(
      parseLeaderboardConfig({
        VITE_CTHUWU_GRAPHQL_ENDPOINT:
          "https://gateway.example/{api-key}/subgraphs/{api-key}/query",
        VITE_CTHUWU_GRAPH_API_KEY: "public-key",
      }).graphEndpoint,
    ).toBe("https://gateway.example/public-key/subgraphs/public-key/query");
  });

  it("rejects insecure, credential-bearing, and unresolved endpoints", () => {
    expect(() =>
      parseLeaderboardConfig({ VITE_CTHUWU_GRAPHQL_ENDPOINT: "http://example.test/graphql" }),
    ).toThrow("HTTPS");
    expect(() =>
      parseLeaderboardConfig({ VITE_CTHUWU_GRAPHQL_ENDPOINT: "https://user:pass@example.test" }),
    ).toThrow("credential-free");
    expect(() =>
      parseLeaderboardConfig({
        VITE_CTHUWU_GRAPHQL_ENDPOINT: "https://example.test/{api-key}/graphql",
      }),
    ).toThrow("unresolved");
  });

  it("uses safe gateway defaults when optional build variables are unset or empty", () => {
    const config = parseLeaderboardConfig({
      VITE_CTHUWU_IPFS_GATEWAY: "",
      VITE_CTHUWU_ARWEAVE_GATEWAY: "",
    });
    expect(config.graphEndpoint).toBeUndefined();
    expect(config.ipfsGateway).toMatch(/^https:/u);
    expect(config.arweaveGateway).toMatch(/^https:/u);
  });
});
