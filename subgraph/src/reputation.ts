import { BigInt } from "@graphprotocol/graph-ts";
import {
  FeedbackRevoked,
  NewFeedback,
  ResponseAppended,
} from "../generated/ReputationRegistry/ReputationRegistry";
import { Feedback, FeedbackResponse } from "../generated/schema";
import {
  ONE,
  REPUTATION_REGISTRY,
  boundedPublicString,
  provenance,
} from "./constants";
import { getOrCreateTentacle } from "./models";

const MAX_TAG = 128;
const MAX_ENDPOINT = 512;
const MAX_URI = 2048;

function feedbackId(agentId: BigInt, client: string, index: BigInt): string {
  return agentId.toString() + ":" + client.toLowerCase() + ":" + index.toString();
}

export function handleNewFeedback(event: NewFeedback): void {
  const tentacle = getOrCreateTentacle(
    event.params.agentId,
    event.block,
    event.transaction.hash,
  );
  const id = feedbackId(
    event.params.agentId,
    event.params.clientAddress.toHexString(),
    event.params.feedbackIndex,
  );
  if (Feedback.load(id) != null) return;

  const tag1 = boundedPublicString(event.params.tag1, MAX_TAG);
  const tag2 = boundedPublicString(event.params.tag2, MAX_TAG);
  const endpoint = boundedPublicString(event.params.endpoint, MAX_ENDPOINT);
  const uri = boundedPublicString(event.params.feedbackURI, MAX_URI);

  const feedback = new Feedback(id);
  feedback.tentacle = tentacle.id;
  feedback.clientAddress = event.params.clientAddress;
  feedback.feedbackIndex = event.params.feedbackIndex;
  feedback.value = event.params.value;
  feedback.valueDecimals = event.params.valueDecimals;
  feedback.tag1 = tag1;
  feedback.tag2 = tag2;
  feedback.endpoint = endpoint;
  feedback.feedbackURI = uri;
  feedback.feedbackHash = event.params.feedbackHash;
  feedback.fieldsRejected =
    tag1 != event.params.tag1 ||
    tag2 != event.params.tag2 ||
    endpoint != event.params.endpoint ||
    uri != event.params.feedbackURI;
  feedback.isRevoked = false;
  feedback.createdBlock = event.block.number;
  feedback.createdTimestamp = event.block.timestamp;
  feedback.createdTransaction = event.transaction.hash;
  feedback.createdLogIndex = event.logIndex;
  feedback.provenance = provenance(
    REPUTATION_REGISTRY,
    event.transaction.hash,
    event.logIndex,
  );
  feedback.save();

  tentacle.feedbackCount = tentacle.feedbackCount.plus(ONE);
  tentacle.activeFeedbackCount = tentacle.activeFeedbackCount.plus(ONE);
  tentacle.save();
}

export function handleFeedbackRevoked(event: FeedbackRevoked): void {
  const id = feedbackId(
    event.params.agentId,
    event.params.clientAddress.toHexString(),
    event.params.feedbackIndex,
  );
  const feedback = Feedback.load(id);
  assert(feedback != null, "revocation references missing feedback");
  if (feedback == null || feedback.isRevoked) return;
  feedback.isRevoked = true;
  feedback.revokedBlock = event.block.number;
  feedback.revokedTimestamp = event.block.timestamp;
  feedback.revokedTransaction = event.transaction.hash;
  feedback.revokedLogIndex = event.logIndex;
  feedback.revocationProvenance = provenance(
    REPUTATION_REGISTRY,
    event.transaction.hash,
    event.logIndex,
  );
  feedback.save();

  const tentacle = getOrCreateTentacle(
    event.params.agentId,
    event.block,
    event.transaction.hash,
  );
  assert(tentacle.activeFeedbackCount.gt(BigInt.zero()), "feedback counter underflow");
  tentacle.activeFeedbackCount = tentacle.activeFeedbackCount.minus(ONE);
  tentacle.revokedFeedbackCount = tentacle.revokedFeedbackCount.plus(ONE);
  tentacle.save();
}

export function handleResponseAppended(event: ResponseAppended): void {
  const feedbackKey = feedbackId(
    event.params.agentId,
    event.params.clientAddress.toHexString(),
    event.params.feedbackIndex,
  );
  const feedback = Feedback.load(feedbackKey);
  assert(feedback != null, "response references missing feedback");
  if (feedback == null) return;

  const response = new FeedbackResponse(
    event.transaction.hash.toHexString() + ":" + event.logIndex.toString(),
  );
  response.feedback = feedbackKey;
  response.responder = event.params.responder;
  response.responseURI = boundedPublicString(event.params.responseURI, MAX_URI);
  response.responseHash = event.params.responseHash;
  response.blockNumber = event.block.number;
  response.timestamp = event.block.timestamp;
  response.transaction = event.transaction.hash;
  response.logIndex = event.logIndex;
  response.provenance = provenance(
    REPUTATION_REGISTRY,
    event.transaction.hash,
    event.logIndex,
  );
  response.save();
}
