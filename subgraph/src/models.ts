import { Address, BigInt, Bytes, ethereum } from "@graphprotocol/graph-ts";
import { Tentacle, Wallet } from "../generated/schema";
import { EMPTY_BYTES, ZERO, ZERO_BYTES } from "./constants";

export function walletId(address: Address): string {
  return address.toHexString().toLowerCase();
}

export function getOrCreateWallet(
  address: Address,
  block: ethereum.Block,
): Wallet {
  const id = walletId(address);
  let wallet = Wallet.load(id);
  if (wallet == null) {
    wallet = new Wallet(id);
    wallet.address = address;
    wallet.rawBalance = ZERO;
    wallet.updatedBlock = block.number;
    wallet.updatedTimestamp = block.timestamp;
    wallet.save();
  }
  return wallet;
}

export function getOrCreateTentacle(
  agentId: BigInt,
  block: ethereum.Block,
  transactionHash: Bytes,
): Tentacle {
  const id = agentId.toString();
  let tentacle = Tentacle.load(id);
  if (tentacle == null) {
    tentacle = new Tentacle(id);
    tentacle.agentId = agentId;
    tentacle.owner = ZERO_BYTES;
    tentacle.approvedOperator = ZERO_BYTES;
    tentacle.agentURI = "";
    tentacle.agentWallet = ZERO_BYTES;
    tentacle.allegiance = EMPTY_BYTES;
    tentacle.protocol = EMPTY_BYTES;
    tentacle.isTentacle = false;
    tentacle.isWalletVerified = false;
    tentacle.registrationBlock = block.number;
    tentacle.registrationTimestamp = block.timestamp;
    tentacle.registrationTransaction = transactionHash;
    tentacle.profileUpdatedBlock = block.number;
    tentacle.profileUpdatedTimestamp = block.timestamp;
    tentacle.metadataUpdatedBlock = block.number;
    tentacle.metadataUpdatedTimestamp = block.timestamp;
    tentacle.feedbackCount = ZERO;
    tentacle.activeFeedbackCount = ZERO;
    tentacle.revokedFeedbackCount = ZERO;
    tentacle.save();
  }
  return tentacle;
}
