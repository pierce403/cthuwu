import {
  createBackend,
  getInboxIdForIdentifier,
  IdentifierKind,
} from "@xmtp/node-sdk";
import { createPublicClient, getAddress, http, isAddress } from "viem";
import { mainnet } from "viem/chains";
import { normalize } from "viem/ens";

const MAX_ENS_NAME_LENGTH = 255;
type XmtpEnvironment = "local" | "dev" | "production";

export type OperatorIdentityResolution = {
  address: `0x${string}`;
  inboxId: string;
};

type ResolveEns = (name: string) => Promise<`0x${string}` | null>;
type ResolveInbox = (
  address: `0x${string}`,
  environment: XmtpEnvironment,
) => Promise<string | null>;

async function resolveEnsWithMainnet(name: string): Promise<`0x${string}` | null> {
  const client = createPublicClient({
    chain: mainnet,
    transport: http(),
  });
  return client.getEnsAddress({ name: normalize(name) });
}

async function resolveInboxWithXmtp(
  address: `0x${string}`,
  environment: XmtpEnvironment,
): Promise<string | null> {
  const backend = await createBackend({ env: environment });
  // This queries XMTP's registered identity association. generateInboxId would
  // also produce an ID for an address that has never created an XMTP inbox.
  return getInboxIdForIdentifier(backend, {
    identifier: address.toLowerCase(),
    identifierKind: IdentifierKind.Ethereum,
  });
}

export async function resolveOperatorIdentity(
  input: string,
  environment: XmtpEnvironment,
  dependencies: {
    resolveEns?: ResolveEns;
    resolveInbox?: ResolveInbox;
  } = {},
): Promise<OperatorIdentityResolution> {
  const candidate = input.trim();
  if (candidate.length === 0) {
    throw new Error("operator identity must be an ENS name or Ethereum address");
  }

  let address: `0x${string}`;
  if (isAddress(candidate, { strict: true })) {
    address = getAddress(candidate);
  } else {
    if (
      candidate.length > MAX_ENS_NAME_LENGTH ||
      !candidate.toLowerCase().endsWith(".eth")
    ) {
      throw new Error("operator identity must be a full 0x Ethereum address or .eth ENS name");
    }
    let normalizedName: string;
    try {
      normalizedName = normalize(candidate);
    } catch (error) {
      throw new Error("operator ENS name is invalid", { cause: error });
    }
    const resolved = await (dependencies.resolveEns ?? resolveEnsWithMainnet)(
      normalizedName,
    );
    if (resolved === null) {
      throw new Error(`ENS name ${JSON.stringify(normalizedName)} has no Ethereum address`);
    }
    address = getAddress(resolved);
  }

  if (address === "0x0000000000000000000000000000000000000000") {
    throw new Error("operator identity must not resolve to the zero address");
  }

  const inboxId = await (dependencies.resolveInbox ?? resolveInboxWithXmtp)(
    address,
    environment,
  );
  if (inboxId === null) {
    throw new Error(
      `Ethereum address ${address} has no inbox on XMTP ${environment}`,
    );
  }
  if (!/^[0-9a-f]{64}$/u.test(inboxId.toLowerCase())) {
    throw new Error("XMTP returned an invalid canonical inbox ID");
  }

  return { address, inboxId: inboxId.toLowerCase() };
}
