import {
  Bytes,
  JSONValue,
  JSONValueKind,
  TypedMap,
  crypto,
  dataSource,
  json,
} from "@graphprotocol/graph-ts";
import { TentacleProfile } from "../generated/schema";
import { boundedPublicString } from "./constants";

const REGISTRATION_SCHEMA =
  "https://eips.ethereum.org/EIPS/eip-8004#registration-v1";
const MAX_DOCUMENT_BYTES = 32768;
const MAX_DEPTH = 6;
const MAX_OBJECT_FIELDS = 32;
const MAX_ARRAY_ITEMS = 16;
const MAX_NAME = 128;
const MAX_DESCRIPTION = 512;
const MAX_URL = 2048;

function safeUrl(value: string, allowServiceSchemes: bool = false): string {
  const bounded = boundedPublicString(value, MAX_URL);
  if (bounded.length == 0) return "";
  if (
    bounded.startsWith("https://") ||
    bounded.startsWith("ipfs://") ||
    bounded.startsWith("ar://")
  ) {
    return bounded;
  }
  if (
    allowServiceSchemes &&
    bounded.startsWith("cthuwu://")
  ) {
    return bounded;
  }
  return "";
}

function canonicalXmtpEndpoint(value: string): string {
  const bounded = boundedPublicString(value, MAX_URL);
  const prefix = "xmtp://";
  if (bounded.length != prefix.length + 64 || !bounded.startsWith(prefix)) {
    return "";
  }
  for (let i = prefix.length; i < bounded.length; i++) {
    const code = bounded.charCodeAt(i);
    const isDigit = code >= 0x30 && code <= 0x39;
    const isLowerHex = code >= 0x61 && code <= 0x66;
    if (!isDigit && !isLowerHex) return "";
  }
  return bounded;
}

function validateTree(value: JSONValue, depth: i32): bool {
  if (depth > MAX_DEPTH) return false;
  if (value.kind == JSONValueKind.STRING) {
    return boundedPublicString(value.toString(), 4096).length == value.toString().length;
  }
  if (value.kind == JSONValueKind.ARRAY) {
    const values = value.toArray();
    if (values.length > MAX_ARRAY_ITEMS) return false;
    for (let i = 0; i < values.length; i++) {
      if (!validateTree(values[i], depth + 1)) return false;
    }
  } else if (value.kind == JSONValueKind.OBJECT) {
    const object = value.toObject();
    if (object.entries.length > MAX_OBJECT_FIELDS) return false;
    for (let i = 0; i < object.entries.length; i++) {
      if (object.entries[i].key.length > 128) return false;
      if (!validateTree(object.entries[i].value, depth + 1)) return false;
    }
  }
  return true;
}

function requiredString(
  object: TypedMap<string, JSONValue>,
  key: string,
  maxLength: i32,
): string {
  const value = object.get(key);
  if (value == null || value.kind != JSONValueKind.STRING) return "";
  return boundedPublicString(value.toString(), maxLength);
}

function requiredBool(
  object: TypedMap<string, JSONValue>,
  key: string,
): JSONValue | null {
  const value = object.get(key);
  if (value == null || value.kind != JSONValueKind.BOOL) return null;
  return value;
}

function validateRegistrations(value: JSONValue | null): i32 {
  if (value == null || value.kind != JSONValueKind.ARRAY) return -1;
  const registrations = value.toArray();
  if (registrations.length == 0 || registrations.length > 8) return -1;
  for (let i = 0; i < registrations.length; i++) {
    if (registrations[i].kind != JSONValueKind.OBJECT) return -1;
    const registration = registrations[i].toObject();
    if (registration.entries.length > 8) return -1;
    const agentRegistry = registration.get("agentRegistry");
    const agentId = registration.get("agentId");
    if (
      agentRegistry == null ||
      agentRegistry.kind != JSONValueKind.STRING ||
      boundedPublicString(agentRegistry.toString(), 256).length == 0 ||
      agentId == null ||
      (agentId.kind != JSONValueKind.NUMBER &&
        agentId.kind != JSONValueKind.STRING)
    ) {
      return -1;
    }
  }
  return registrations.length;
}

function validateSupportedTrust(value: JSONValue | null): bool {
  if (value == null) return true;
  if (value.kind != JSONValueKind.ARRAY) return false;
  const trust = value.toArray();
  if (trust.length > 8) return false;
  for (let i = 0; i < trust.length; i++) {
    if (
      trust[i].kind != JSONValueKind.STRING ||
      boundedPublicString(trust[i].toString(), 64).length == 0
    ) {
      return false;
    }
  }
  return true;
}

function parseServices(
  value: JSONValue | null,
  profile: TentacleProfile,
): i32 {
  if (value == null || value.kind != JSONValueKind.ARRAY) return -1;
  const services = value.toArray();
  if (services.length > MAX_ARRAY_ITEMS) return -1;
  for (let i = 0; i < services.length; i++) {
    if (services[i].kind != JSONValueKind.OBJECT) return -1;
    const service = services[i].toObject();
    if (service.entries.length > 8) return -1;
    const name = requiredString(service, "name", 64);
    const endpointValue = service.get("endpoint");
    if (
      name.length == 0 ||
      endpointValue == null ||
      endpointValue.kind != JSONValueKind.STRING
    ) {
      return -1;
    }
    const endpointText = endpointValue.toString();
    if (name == "CTHUWU-XMTP" || name == "XMTP") {
      const xmtpEndpoint = canonicalXmtpEndpoint(endpointText);
      // An ERC-8004 service name is arbitrary public input. Keep the profile,
      // but expose no XMTP route unless it exactly matches our documented
      // production inbox URI convention.
      if (xmtpEndpoint.length > 0) profile.xmtpEndpoint = xmtpEndpoint;
      continue;
    }
    const endpoint = safeUrl(endpointText, true);
    if (endpoint.length == 0) return -1;
    if (name == "CTHUWU") profile.cthuwuEndpoint = endpoint;
  }
  return services.length;
}

export function handleProfile(content: Bytes): void {
  const context = dataSource.context();
  const id = context.getString("profileId");
  if (TentacleProfile.load(id) != null) return;

  const profile = new TentacleProfile(id);
  profile.sourceURI = context.getString("sourceURI");
  profile.contentHash = Bytes.fromByteArray(crypto.keccak256(content));
  profile.byteLength = content.length;
  profile.parseValid = false;

  if (content.length == 0 || content.length > MAX_DOCUMENT_BYTES) {
    profile.save();
    return;
  }

  const parsed = json.try_fromBytes(content);
  if (parsed.isError || parsed.value.kind != JSONValueKind.OBJECT) {
    profile.save();
    return;
  }
  if (!validateTree(parsed.value, 0)) {
    profile.save();
    return;
  }

  const object = parsed.value.toObject();
  const schemaType = requiredString(object, "type", 256);
  const name = requiredString(object, "name", MAX_NAME);
  const description = requiredString(object, "description", MAX_DESCRIPTION);
  const imageValue = requiredString(object, "image", MAX_URL);
  const image = safeUrl(imageValue);
  const active = requiredBool(object, "active");
  const x402Support = requiredBool(object, "x402Support");
  const serviceCount = parseServices(object.get("services"), profile);
  const registrationCount = validateRegistrations(object.get("registrations"));

  if (
    schemaType != REGISTRATION_SCHEMA ||
    name.length == 0 ||
    description.length == 0 ||
    image.length == 0 ||
    active == null ||
    x402Support == null ||
    serviceCount < 0 ||
    registrationCount < 0 ||
    !validateSupportedTrust(object.get("supportedTrust"))
  ) {
    profile.save();
    return;
  }

  profile.schemaType = schemaType;
  profile.name = name;
  profile.description = description;
  profile.image = image;
  profile.active = active.toBool();
  profile.x402Support = x402Support.toBool();
  profile.serviceCount = serviceCount;
  profile.registrationCount = registrationCount;
  profile.parseValid = true;
  profile.save();
}
