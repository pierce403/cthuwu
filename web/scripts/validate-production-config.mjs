import { pathToFileURL } from "node:url";

const PLACEHOLDER = "{api-key}";
const API_KEY = /^[A-Za-z0-9._~-]{8,256}$/u;

export function validateProductionConfig(environment) {
  const template = environment.VITE_CTHUWU_GRAPHQL_ENDPOINT?.trim() ?? "";
  const apiKey = environment.VITE_CTHUWU_GRAPH_API_KEY?.trim() ?? "";
  if (!template || template.includes("REPLACE_ME")) {
    throw new Error("VITE_CTHUWU_GRAPHQL_ENDPOINT must identify the deployed production subgraph");
  }
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
    console.log("Production leaderboard configuration is valid.");
  } catch (error) {
    console.error(error instanceof Error ? error.message : "Production configuration is invalid");
    process.exitCode = 1;
  }
}
