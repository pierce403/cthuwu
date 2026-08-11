import type { TentacleProfile } from "./leaderboard-types";

const REGISTRATION_TYPE = "https://eips.ethereum.org/EIPS/eip-8004#registration-v1";
const MAX_AGENT_URI_BYTES = 24 * 1024;
const MAX_PROFILE_JSON_BYTES = 16 * 1024;
const MAX_NAME = 128;
const MAX_DESCRIPTION = 512;
const MAX_URI = 2_048;
const MAX_SERVICES = 16;
const MAX_JSON_DEPTH = 8;
const MAX_JSON_FIELDS = 128;
const MAX_JSON_ARRAY_ITEMS = 64;
const MAX_JSON_STRING = 4_096;

export function fallbackProfile(agentId: string, tentacleId?: string): TentacleProfile {
  return {
    name: safeText(tentacleId, MAX_NAME) ?? `Tentacle #${agentId}`,
    active: true,
    sourceUri: "on-chain fallback",
  };
}

export function parseDataRegistration(
  agentUri: string,
  agentId: string,
  tentacleId?: string,
): TentacleProfile | undefined {
  if (new TextEncoder().encode(agentUri).length > MAX_AGENT_URI_BYTES) return undefined;
  const prefix = "data:application/json;base64,";
  if (!agentUri.startsWith(prefix)) return undefined;
  let decoded: string;
  try {
    const encoded = agentUri.slice(prefix.length);
    if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(encoded)) {
      return undefined;
    }
    const binary = atob(encoded);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return undefined;
  }
  if (new TextEncoder().encode(decoded).length > MAX_PROFILE_JSON_BYTES) return undefined;

  let parsed: unknown;
  try {
    parsed = JSON.parse(decoded);
  } catch {
    return undefined;
  }
  if (!hasBoundedJsonShape(parsed) || !isRecord(parsed) || parsed.type !== REGISTRATION_TYPE) {
    return undefined;
  }
  const name = safeText(parsed.name, MAX_NAME);
  const description = safeText(parsed.description, MAX_DESCRIPTION);
  const image = safeUri(parsed.image);
  const active = parsed.active === true;
  if (Array.isArray(parsed.services) && parsed.services.length > MAX_SERVICES) return undefined;
  const services = Array.isArray(parsed.services) ? parsed.services : [];
  let xmtpEndpoint: string | undefined;
  let cthuwuEndpoint: string | undefined;
  for (const service of services) {
    if (!isRecord(service)) continue;
    const serviceName = safeText(service.name, 32);
    const endpoint = safeText(service.endpoint, MAX_URI);
    if (!endpoint) continue;
    if (
      (serviceName === "CTHUWU-XMTP" || serviceName === "XMTP") &&
      isSafeXmtpEndpoint(endpoint)
    ) {
      xmtpEndpoint = endpoint;
    }
    if (serviceName === "CTHUWU" && isSafePublicEndpoint(endpoint)) cthuwuEndpoint = endpoint;
  }
  return {
    name: name ?? fallbackProfile(agentId, tentacleId).name,
    ...(description ? { description } : {}),
    ...(image ? { image } : {}),
    active,
    ...(xmtpEndpoint ? { xmtpEndpoint } : {}),
    ...(cthuwuEndpoint ? { cthuwuEndpoint } : {}),
    sourceUri: "data:application/json;base64",
  };
}

function safeText(value: unknown, maximum: number): string | undefined {
  if (typeof value !== "string" || value.length === 0 || value.length > maximum) return undefined;
  if ([...value].some((character) => isUnsafeControl(character.codePointAt(0) ?? 0))) return undefined;
  return value;
}

function safeUri(value: unknown): string | undefined {
  const uri = safeText(value, MAX_URI);
  if (!uri) return undefined;
  if (uri.startsWith("ipfs://") || uri.startsWith("ar://")) return uri;
  try {
    const parsed = new URL(uri);
    return parsed.protocol === "https:" && !parsed.username && !parsed.password
      ? parsed.href
      : undefined;
  } catch {
    return undefined;
  }
}

function isSafeXmtpEndpoint(value: string): boolean {
  return /^xmtp:\/\/[0-9a-f]{64}$/u.test(value);
}

function isSafePublicEndpoint(value: string): boolean {
  if (value.startsWith("ipfs://") || value.startsWith("ar://")) return value.length <= MAX_URI;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "https:" && !parsed.username && !parsed.password;
  } catch {
    return false;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isUnsafeControl(codePoint: number): boolean {
  return (
    (codePoint >= 0 && codePoint <= 0x1f) ||
    (codePoint >= 0x7f && codePoint <= 0x9f) ||
    codePoint === 0x061c ||
    (codePoint >= 0x200b && codePoint <= 0x200f) ||
    (codePoint >= 0x202a && codePoint <= 0x202e) ||
    (codePoint >= 0x2060 && codePoint <= 0x206f) ||
    codePoint === 0xfeff
  );
}


function hasBoundedJsonShape(root: unknown): boolean {
  let fields = 0;
  let arrayItems = 0;
  const visit = (value: unknown, depth: number): boolean => {
    if (depth > MAX_JSON_DEPTH) return false;
    if (typeof value === "string") return value.length <= MAX_JSON_STRING;
    if (value === null || typeof value === "number" || typeof value === "boolean") return true;
    if (Array.isArray(value)) {
      arrayItems += value.length;
      return arrayItems <= MAX_JSON_ARRAY_ITEMS && value.every((item) => visit(item, depth + 1));
    }
    if (!isRecord(value)) return false;
    const entries = Object.entries(value);
    fields += entries.length;
    return (
      fields <= MAX_JSON_FIELDS &&
      entries.every(([key, item]) => key.length <= 64 && visit(item, depth + 1))
    );
  };
  return visit(root, 0);
}
