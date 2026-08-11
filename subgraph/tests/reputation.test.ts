import { afterEach, assert, clearStore, describe, test } from "matchstick-as";
import {
  handleFeedbackRevoked,
  handleNewFeedback,
  handleResponseAppended,
} from "../src/reputation";
import { WALLET_TWO, feedback, response, revoked } from "./helpers";

afterEach(() => clearStore());

describe("ERC-8004 reputation provenance", () => {
  test("indexes signed feedback, revocation, and response without ranking", () => {
    handleNewFeedback(feedback(9));
    const id = "9:" + WALLET_TWO.toHexString() + ":1";
    assert.fieldEquals("Feedback", id, "value", "-15");
    assert.fieldEquals("Feedback", id, "valueDecimals", "1");
    assert.fieldEquals("Feedback", id, "isRevoked", "false");
    assert.fieldEquals("Tentacle", "9", "activeFeedbackCount", "1");

    handleResponseAppended(response(9));
    assert.entityCount("FeedbackResponse", 1);

    const revocation = revoked(9);
    handleFeedbackRevoked(revocation);
    assert.fieldEquals("Feedback", id, "isRevoked", "true");
    assert.fieldEquals("Tentacle", "9", "activeFeedbackCount", "0");
    assert.fieldEquals("Tentacle", "9", "revokedFeedbackCount", "1");
    assert.fieldEquals(
      "Feedback",
      id,
      "revocationProvenance",
      "eip155:8453:0x8004baa17c55a88189ae136b182e5fda19de9b63:" +
        revocation.transaction.hash.toHexString() +
        ":" +
        revocation.logIndex.toString(),
    );
  });
});
