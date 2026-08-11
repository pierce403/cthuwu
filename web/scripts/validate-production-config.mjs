import { pathToFileURL } from "node:url";

const PLACEHOLDER = "{api-key}";
const API_KEY = /^[A-Za-z0-9._~-]{8,256}$/u;
const ADDRESS = /^0x[0-9a-f]{40}$/u;
const ZERO_ADDRESS = `0x${"0".repeat(40)}`;
const UNSIGNED_DECIMAL = /^(?:0|[1-9][0-9]*)$/u;
const AGENT0 = "https://gateway.thegraph.com/api/{api-key}/subgraphs/id/43s9hQRurMGjuYnC1r2ZwS6xSQktbFyXMPMqGKUFJojb";
// This is intentionally public client configuration, restricted at The Graph gateway.
const PUBLIC_AGENT0_KEY = "2636605c8c75cc8a1b8ddb5c07f8c563";

export function validateProductionConfig(environment) {
  const template = environment.VITE_CTHUWU_GRAPHQL_ENDPOINT?.trim() || AGENT0;
  const apiKey = environment.VITE_CTHUWU_GRAPH_API_KEY?.trim() || PUBLIC_AGENT0_KEY;
  if (template.includes("REPLACE_ME")) throw new Error("Graph endpoint contains REPLACE_ME");
  if (
    template.includes(PLACEHOLDER) &&
    (!API_KEY.test(apiKey) || apiKey === "REPLACE_ME")
  ) {
    throw new Error(
      "VITE_CTHUWU_GRAPH_API_KEY must resolve the endpoint placeholder with a URL-safe public key",
    );
  }
  const resolved = template.replaceAll(PLACEHOLDER, apiKey);
  if (resolved.includes("{") || resolved.includes("}")) {
    throw new Error("VITE_CTHUWU_GRAPHQL_ENDPOINT contains an unresolved placeholder");
  }
  validateHttpsUrl(resolved, "VITE_CTHUWU_GRAPHQL_ENDPOINT");
  validateHttpsUrl(environment.VITE_CTHUWU_BASE_RPC_ENDPOINT?.trim() || "https://mainnet.base.org/", "VITE_CTHUWU_BASE_RPC_ENDPOINT");
  for (const [name, value] of [
    ["VITE_CTHUWU_IPFS_GATEWAY", environment.VITE_CTHUWU_IPFS_GATEWAY],
    ["VITE_CTHUWU_ARWEAVE_GATEWAY", environment.VITE_CTHUWU_ARWEAVE_GATEWAY],
  ]) {
    if (value?.trim()) validateHttpsUrl(value.trim(), name);
  }
  const freshness = environment.VITE_CTHUWU_LEADERBOARD_FRESH_MS?.trim();
  if (freshness) {
    const milliseconds = Number(freshness);
    if (!Number.isSafeInteger(milliseconds) || milliseconds < 60_000 || milliseconds > 86_400_000) {
      throw new Error("VITE_CTHUWU_LEADERBOARD_FRESH_MS must be between 60000 and 86400000");
    }
  }
  const brandingContract = environment.VITE_CTHUWU_BRANDING_CONTRACT;
  if (
    brandingContract !== undefined && brandingContract !== "" &&
    (!ADDRESS.test(brandingContract) || brandingContract === ZERO_ADDRESS)
  ) {
    throw new Error(
      "VITE_CTHUWU_BRANDING_CONTRACT must be a lowercase nonzero 0x-prefixed address",
    );
  }
  const assignmentRefresh = environment.VITE_CTHUWU_ASSIGNMENT_REFRESH_MS;
  if (assignmentRefresh !== undefined && assignmentRefresh !== "") {
    if (!UNSIGNED_DECIMAL.test(assignmentRefresh)) {
      throw new Error(
        "VITE_CTHUWU_ASSIGNMENT_REFRESH_MS must be an integer between 60000 and 3600000",
      );
    }
    const milliseconds = Number(assignmentRefresh);
    if (
      !Number.isSafeInteger(milliseconds) ||
      milliseconds < 60_000 ||
      milliseconds > 3_600_000
    ) {
      throw new Error(
        "VITE_CTHUWU_ASSIGNMENT_REFRESH_MS must be an integer between 60000 and 3600000",
      );
    }
  }
  return resolved;
}

function validateHttpsUrl(value, name) {
  if (value.length > 2_048) throw new Error(`${name} is too long`);
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${name} must be an absolute HTTPS URL`);
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
    throw new Error(`${name} must be an absolute credential-free HTTPS URL`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    validateProductionConfig(process.env);
    console.log("Production web configuration is valid.");
  } catch (error) {
    console.error(error instanceof Error ? error.message : "Production configuration is invalid");
    process.exitCode = 1;
  }
}
