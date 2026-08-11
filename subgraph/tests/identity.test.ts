import { Address, Bytes } from "@graphprotocol/graph-ts";
import { afterEach, assert, clearStore, describe, test } from "matchstick-as";
import {
  handleMetadataSet,
  handleRegistered,
  handleTransfer,
  handleURIUpdated,
} from "../src/identity";
import {
  OWNER,
  WALLET,
  WALLET_TWO,
  ZERO,
  identityTransfer,
  metadata,
  registered,
  uriUpdated,
} from "./helpers";
import { Tentacle } from "../generated/schema";

afterEach(() => clearStore());

describe("ERC-8004 identity current state", () => {
  test("registers one durable Tentacle record and updates URI", () => {
    handleTransfer(identityTransfer(7, ZERO, OWNER));
    handleRegistered(registered(7, "data:application/json;base64,e30="));
    assert.entityCount("Tentacle", 1);
    assert.fieldEquals("Tentacle", "7", "owner", OWNER.toHexString());
    assert.fieldEquals("Tentacle", "7", "agentURI", "data:application/json;base64,e30=");

    handleURIUpdated(uriUpdated(7, "ipfs://bafybeigdyrztfixtureaaaaaaaaaaaaaaaa"));
    assert.fieldEquals(
      "Tentacle",
      "7",
      "agentURI",
      "ipfs://bafybeigdyrztfixtureaaaaaaaaaaaaaaaa",
    );
  });

  test("retains the runtime URI budget and rejects larger values", () => {
    let bounded = "data:application/json;base64,";
    while (bounded.length < 8192) bounded += "a";
    handleRegistered(registered(8, bounded));
    const accepted = Tentacle.load("8");
    assert.assertNotNull(accepted);
    assert.i32Equals(accepted!.agentURI.length, 8192);

    handleURIUpdated(uriUpdated(8, bounded + "a"));
    assert.fieldEquals("Tentacle", "8", "agentURI", "");
  });

  test("uses byte-exact current allegiance for opt-in and opt-out", () => {
    handleRegistered(registered(1));
    handleMetadataSet(
      metadata(1, "cthuwu.allegiance", Bytes.fromUTF8("uwu-tentacle-v1")),
    );
    assert.fieldEquals("Tentacle", "1", "isTentacle", "true");

    handleMetadataSet(
      metadata(1, "cthuwu.allegiance", Bytes.fromUTF8("UWU-TENTACLE-V1")),
    );
    assert.fieldEquals("Tentacle", "1", "isTentacle", "false");

    handleMetadataSet(
      metadata(1, "cthuwu.allegiance", Bytes.fromHexString("0x")),
    );
    assert.fieldEquals("Tentacle", "1", "allegiance", "0x");
    assert.fieldEquals("Tentacle", "1", "isTentacle", "false");
  });

  test("suspends an identity when agentWallet clears or transfers", () => {
    handleRegistered(registered(2));
    handleMetadataSet(metadata(2, "agentWallet", WALLET));
    assert.fieldEquals("Tentacle", "2", "isWalletVerified", "true");
    assert.fieldEquals("Tentacle", "2", "wallet", WALLET.toHexString());

    handleMetadataSet(metadata(2, "agentWallet", Bytes.fromHexString("0x")));
    assert.fieldEquals("Tentacle", "2", "isWalletVerified", "false");
    assert.fieldEquals(
      "Tentacle",
      "2",
      "agentWallet",
      "0x0000000000000000000000000000000000000000",
    );

    handleMetadataSet(metadata(2, "agentWallet", WALLET));
    handleTransfer(identityTransfer(2, OWNER, WALLET_TWO));
    assert.fieldEquals("Tentacle", "2", "owner", WALLET_TWO.toHexString());
    assert.fieldEquals("Tentacle", "2", "isWalletVerified", "false");
  });

  test("retains multiple identities sharing one Wallet relation", () => {
    handleRegistered(registered(3));
    handleRegistered(registered(4));
    handleMetadataSet(metadata(3, "agentWallet", WALLET));
    handleMetadataSet(metadata(4, "agentWallet", WALLET));
    assert.entityCount("Tentacle", 2);
    assert.entityCount("Wallet", 1);
    assert.fieldEquals("Tentacle", "3", "wallet", WALLET.toHexString());
    assert.fieldEquals("Tentacle", "4", "wallet", WALLET.toHexString());
    assert.fieldEquals("Wallet", WALLET.toHexString(), "rawBalance", "0");
  });

  test("bounds hostile metadata and keeps it non-membership", () => {
    handleRegistered(registered(5));
    const oversized = new Bytes(257);
    handleMetadataSet(metadata(5, "cthuwu.allegiance", oversized));
    assert.fieldEquals("Tentacle", "5", "isTentacle", "false");
    assert.fieldEquals(
      "TentacleMetadata",
      "5:cthuwu.allegiance",
      "valueTooLarge",
      "true",
    );
  });

  test("ignores unrelated metadata before decoding and validates Tentacle IDs", () => {
    handleRegistered(registered(6));
    handleMetadataSet(
      metadata(6, "cthuwu.allegiance-lookalike", Bytes.fromUTF8("uwu-tentacle-v1")),
    );
    assert.entityCount("TentacleMetadata", 0);
    assert.fieldEquals("Tentacle", "6", "isTentacle", "false");

    handleMetadataSet(
      metadata(6, "cthuwu.tentacle-id", Bytes.fromUTF8("tentacle_valid-id")),
    );
    assert.fieldEquals("Tentacle", "6", "tentacleId", "tentacle_valid-id");

    handleMetadataSet(
      metadata(6, "cthuwu.tentacle-id", Bytes.fromHexString("0xff")),
    );
    const tentacle = Tentacle.load("6");
    assert.assertNotNull(tentacle);
    assert.assertNull(tentacle!.tentacleId);
  });
});
