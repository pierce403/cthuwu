import { Address, Bytes, DataSourceContext } from "@graphprotocol/graph-ts";
import {
  Approval,
  ApprovalForAll,
  MetadataSet,
  Registered,
  Transfer,
  URIUpdated,
} from "../generated/IdentityRegistry/IdentityRegistry";
import {
  IpfsTentacleProfile,
  ArweaveTentacleProfile,
} from "../generated/templates";
import {
  OperatorApproval,
  TentacleMetadata,
} from "../generated/schema";
import {
  AGENT_WALLET_KEY,
  ALLEGIANCE_KEY,
  ALLEGIANCE_VALUE,
  EMPTY_BYTES,
  PROTOCOL_KEY,
  TENTACLE_ID_KEY,
  ZERO_BYTES,
  bytesEqual,
  boundedPublicString,
  isZeroAddress,
} from "./constants";
import { getOrCreateTentacle, getOrCreateWallet } from "./models";

const MAX_AGENT_URI = 8 * 1024;
const MAX_METADATA_VALUE = 256;
const MAX_TENTACLE_ID = 96;
const MAX_TENTACLE_SUFFIX = 64;
const TENTACLE_PREFIX = Bytes.fromUTF8("tentacle_");
const ALLEGIANCE_KEY_HASH =
  "0x41bfe7e2ab5cfab2d8c94d2b438fdf587245fa0056f7ccd479ffebedfb1fc485";
const PROTOCOL_KEY_HASH =
  "0x142ebfd4fce0092b9e54b901c5ea1a2844c82219bea291a6c1640ca14b245a4e";
const TENTACLE_ID_KEY_HASH =
  "0xb2ed9d2542d7ec81e16e990fe1674ddf65d9663a2410b29a9176dd602bbb0b4c";
const AGENT_WALLET_KEY_HASH =
  "0x2ac6109326e720d1435c0db66f7e35eda7839f52b6f1f5520a60788e132b4e39";

function allowedMetadataKey(indexedKey: Bytes): string {
  const hash = indexedKey.toHexString();
  if (hash == ALLEGIANCE_KEY_HASH) return ALLEGIANCE_KEY;
  if (hash == PROTOCOL_KEY_HASH) return PROTOCOL_KEY;
  if (hash == TENTACLE_ID_KEY_HASH) return TENTACLE_ID_KEY;
  if (hash == AGENT_WALLET_KEY_HASH) return AGENT_WALLET_KEY;
  return "";
}

function decodeTentacleId(value: Bytes): string {
  if (
    value.length <= TENTACLE_PREFIX.length ||
    value.length > MAX_TENTACLE_ID ||
    value.length - TENTACLE_PREFIX.length > MAX_TENTACLE_SUFFIX
  ) {
    return "";
  }
  for (let i = 0; i < TENTACLE_PREFIX.length; i++) {
    if (value[i] != TENTACLE_PREFIX[i]) return "";
  }
  let previousSeparator = false;
  for (let i = TENTACLE_PREFIX.length; i < value.length; i++) {
    const byte = value[i];
    const separator = byte == 0x2d || byte == 0x5f;
    const allowed =
      (byte >= 0x61 && byte <= 0x7a) ||
      (byte >= 0x30 && byte <= 0x39) ||
      separator;
    if (
      !allowed ||
      (separator &&
        (i == TENTACLE_PREFIX.length || i + 1 == value.length || previousSeparator))
    ) {
      return "";
    }
    previousSeparator = separator;
  }
  // Every byte was validated as ASCII before using the host UTF-8 conversion.
  return value.toString();
}

function isCid(value: string): bool {
  if (value.length < 32 || value.length > 128) return false;
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    const valid =
      (code >= 0x30 && code <= 0x39) ||
      (code >= 0x41 && code <= 0x5a) ||
      (code >= 0x61 && code <= 0x7a);
    if (!valid) return false;
  }
  return true;
}

function isArweaveId(value: string): bool {
  if (value.length != 43) return false;
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    const valid =
      (code >= 0x30 && code <= 0x39) ||
      (code >= 0x41 && code <= 0x5a) ||
      (code >= 0x61 && code <= 0x7a) ||
      code == 0x2d ||
      code == 0x5f;
    if (!valid) return false;
  }
  return true;
}

function updateProfileReference(uri: string): void {
  if (uri.startsWith("ipfs://")) {
    const cid = uri.substr(7);
    if (!isCid(cid)) return;
    const context = new DataSourceContext();
    context.setString("profileId", "ipfs:" + cid);
    context.setString("sourceURI", uri);
    IpfsTentacleProfile.createWithContext(cid, context);
  } else if (uri.startsWith("ar://")) {
    const transactionId = uri.substr(5);
    if (!isArweaveId(transactionId)) return;
    const context = new DataSourceContext();
    context.setString("profileId", "ar:" + transactionId);
    context.setString("sourceURI", uri);
    ArweaveTentacleProfile.createWithContext(transactionId, context);
  }
}

function setProfile(uri: string): void {
  // The file handler is deliberately separate and imports no chain bindings.
  if (uri.startsWith("ipfs://")) {
    const cid = uri.substr(7);
    if (!isCid(cid)) return;
    updateProfileReference(uri);
  } else if (uri.startsWith("ar://")) {
    const transactionId = uri.substr(5);
    if (!isArweaveId(transactionId)) return;
    updateProfileReference(uri);
  }
}

export function handleRegistered(event: Registered): void {
  const tentacle = getOrCreateTentacle(
    event.params.agentId,
    event.block,
    event.transaction.hash,
  );
  const uri = boundedPublicString(event.params.agentURI, MAX_AGENT_URI);
  tentacle.owner = event.params.owner;
  tentacle.agentURI = uri;
  tentacle.registrationBlock = event.block.number;
  tentacle.registrationTimestamp = event.block.timestamp;
  tentacle.registrationTransaction = event.transaction.hash;
  tentacle.profileUpdatedBlock = event.block.number;
  tentacle.profileUpdatedTimestamp = event.block.timestamp;
  tentacle.unset("profile");
  if (uri.startsWith("ipfs://") && isCid(uri.substr(7))) {
    tentacle.profile = "ipfs:" + uri.substr(7);
    setProfile(uri);
  } else if (uri.startsWith("ar://") && isArweaveId(uri.substr(5))) {
    tentacle.profile = "ar:" + uri.substr(5);
    setProfile(uri);
  }
  tentacle.save();
}

export function handleURIUpdated(event: URIUpdated): void {
  const tentacle = getOrCreateTentacle(
    event.params.agentId,
    event.block,
    event.transaction.hash,
  );
  const uri = boundedPublicString(event.params.newURI, MAX_AGENT_URI);
  const changed = tentacle.agentURI != uri;
  tentacle.agentURI = uri;
  tentacle.profileUpdatedBlock = event.block.number;
  tentacle.profileUpdatedTimestamp = event.block.timestamp;
  tentacle.unset("profile");
  if (uri.startsWith("ipfs://") && isCid(uri.substr(7))) {
    tentacle.profile = "ipfs:" + uri.substr(7);
    if (changed) setProfile(uri);
  } else if (uri.startsWith("ar://") && isArweaveId(uri.substr(5))) {
    tentacle.profile = "ar:" + uri.substr(5);
    if (changed) setProfile(uri);
  }
  tentacle.save();
}

export function handleMetadataSet(event: MetadataSet): void {
  // Filter by the indexed hash before reading the unindexed dynamic string. The
  // pinned implementation emits both from the same input, so a matching topic
  // positively identifies one of the four supported ASCII keys and unrelated
  // hostile metadata never reaches string decoding.
  const key = allowedMetadataKey(event.params.indexedMetadataKey);
  if (key.length == 0) return;

  const tentacle = getOrCreateTentacle(
    event.params.agentId,
    event.block,
    event.transaction.hash,
  );
  const tooLarge = event.params.metadataValue.length > MAX_METADATA_VALUE;
  const value = tooLarge ? EMPTY_BYTES : event.params.metadataValue;
  const metadataId = tentacle.id + ":" + key;
  let metadata = TentacleMetadata.load(metadataId);
  if (metadata == null) metadata = new TentacleMetadata(metadataId);
  metadata.tentacle = tentacle.id;
  metadata.key = key;
  metadata.value = value;
  metadata.valueTooLarge = tooLarge;
  metadata.updatedBlock = event.block.number;
  metadata.updatedTimestamp = event.block.timestamp;
  metadata.updatedTransaction = event.transaction.hash;
  metadata.save();

  if (key == ALLEGIANCE_KEY) {
    tentacle.allegiance = value;
    tentacle.isTentacle = !tooLarge && bytesEqual(value, ALLEGIANCE_VALUE);
  } else if (key == PROTOCOL_KEY) {
    tentacle.protocol = value;
  } else if (key == TENTACLE_ID_KEY) {
    const publicId = tooLarge ? "" : decodeTentacleId(value);
    if (!tooLarge && publicId.length > 0) tentacle.tentacleId = publicId;
    else tentacle.unset("tentacleId");
  } else if (key == AGENT_WALLET_KEY) {
    tentacle.unset("wallet");
    tentacle.agentWallet = ZERO_BYTES;
    tentacle.isWalletVerified = false;
    if (!tooLarge && value.length == 20) {
      const address = Address.fromBytes(value);
      if (!isZeroAddress(address)) {
        const wallet = getOrCreateWallet(address, event.block);
        tentacle.agentWallet = address;
        tentacle.wallet = wallet.id;
        tentacle.isWalletVerified = true;
      }
    }
  }

  tentacle.metadataUpdatedBlock = event.block.number;
  tentacle.metadataUpdatedTimestamp = event.block.timestamp;
  tentacle.save();
}

export function handleTransfer(event: Transfer): void {
  const tentacle = getOrCreateTentacle(
    event.params.tokenId,
    event.block,
    event.transaction.hash,
  );
  tentacle.owner = event.params.to;
  tentacle.approvedOperator = ZERO_BYTES;

  // IdentityRegistry v2 clears agentWallet before every non-mint transfer. Keep
  // the current-state reconstruction fail-closed even if the paired metadata log
  // is missing from a malformed fixture or future incompatible deployment.
  if (!isZeroAddress(event.params.from) && !isZeroAddress(event.params.to)) {
    tentacle.agentWallet = ZERO_BYTES;
    tentacle.isWalletVerified = false;
    tentacle.unset("wallet");
  }
  tentacle.save();
}

export function handleApproval(event: Approval): void {
  const tentacle = getOrCreateTentacle(
    event.params.tokenId,
    event.block,
    event.transaction.hash,
  );
  tentacle.approvedOperator = event.params.approved;
  tentacle.save();
}

export function handleApprovalForAll(event: ApprovalForAll): void {
  const id =
    event.params.owner.toHexString().toLowerCase() +
    ":" +
    event.params.operator.toHexString().toLowerCase();
  let approval = OperatorApproval.load(id);
  if (approval == null) approval = new OperatorApproval(id);
  approval.owner = event.params.owner;
  approval.operator = event.params.operator;
  approval.approved = event.params.approved;
  approval.updatedBlock = event.block.number;
  approval.updatedTimestamp = event.block.timestamp;
  approval.updatedTransaction = event.transaction.hash;
  approval.save();
}
