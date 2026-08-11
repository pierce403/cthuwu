import { Address, BigInt, Bytes } from "@graphprotocol/graph-ts";

export const ZERO_ADDRESS = Address.fromString(
  "0x0000000000000000000000000000000000000000",
);
export const ZERO_BYTES = Bytes.fromHexString(
  "0x0000000000000000000000000000000000000000",
);
export const EMPTY_BYTES = Bytes.fromHexString("0x");

export const ALLEGIANCE_KEY = "cthuwu.allegiance";
export const PROTOCOL_KEY = "cthuwu.protocol";
export const TENTACLE_ID_KEY = "cthuwu.tentacle-id";
export const AGENT_WALLET_KEY = "agentWallet";

export const ALLEGIANCE_VALUE = Bytes.fromUTF8("uwu-tentacle-v1");
export const PROTOCOL_VALUE = Bytes.fromUTF8("1");

export const IDENTITY_REGISTRY =
  "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432";
export const REPUTATION_REGISTRY =
  "0x8004baa17c55a88189ae136b182e5fda19de9b63";
export const CHAIN_ID = "8453";

export const ZERO = BigInt.zero();
export const ONE = BigInt.fromI32(1);

export function bytesEqual(left: Bytes, right: Bytes): bool {
  return left.toHexString() == right.toHexString();
}

export function isZeroAddress(address: Address): bool {
  return address.toHexString() == ZERO_ADDRESS.toHexString();
}

export function hasControlCharacters(value: string): bool {
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (code < 0x20 || code == 0x7f) return true;
  }
  return false;
}

export function boundedPublicString(value: string, maxLength: i32): string {
  if (value.length > maxLength || hasControlCharacters(value)) return "";
  return value;
}

export function provenance(
  registry: string,
  transaction: Bytes,
  logIndex: BigInt,
): string {
  return (
    "eip155:" +
    CHAIN_ID +
    ":" +
    registry +
    ":" +
    transaction.toHexString() +
    ":" +
    logIndex.toString()
  );
}
