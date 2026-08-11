import { Address, BigInt, Bytes, crypto, ethereum } from "@graphprotocol/graph-ts";
import { newMockEvent } from "matchstick-as";
import {
  MetadataSet,
  Registered,
  Transfer as IdentityTransfer,
  URIUpdated,
} from "../generated/IdentityRegistry/IdentityRegistry";
import {
  FeedbackRevoked,
  NewFeedback,
  ResponseAppended,
} from "../generated/ReputationRegistry/ReputationRegistry";
import { Transfer as UwuTransfer } from "../generated/UWU/UWU";

export const ZERO = Address.fromString(
  "0x0000000000000000000000000000000000000000",
);
export const OWNER = Address.fromString(
  "0x1111111111111111111111111111111111111111",
);
export const WALLET = Address.fromString(
  "0x2222222222222222222222222222222222222222",
);
export const WALLET_TWO = Address.fromString(
  "0x3333333333333333333333333333333333333333",
);
export const HASH = Bytes.fromHexString(
  "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
);

export function registered(id: i32, uri: string = ""): Registered {
  const event = changetype<Registered>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam(
      "agentId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("agentURI", ethereum.Value.fromString(uri)),
  );
  event.parameters.push(
    new ethereum.EventParam("owner", ethereum.Value.fromAddress(OWNER)),
  );
  return event;
}

export function metadata(
  id: i32,
  key: string,
  value: Bytes,
): MetadataSet {
  const event = changetype<MetadataSet>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam(
      "agentId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "indexedMetadataKey",
      ethereum.Value.fromBytes(
        Bytes.fromByteArray(crypto.keccak256(Bytes.fromUTF8(key))),
      ),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("metadataKey", ethereum.Value.fromString(key)),
  );
  event.parameters.push(
    new ethereum.EventParam("metadataValue", ethereum.Value.fromBytes(value)),
  );
  return event;
}

export function identityTransfer(
  id: i32,
  from: Address,
  to: Address,
): IdentityTransfer {
  const event = changetype<IdentityTransfer>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam("from", ethereum.Value.fromAddress(from)),
  );
  event.parameters.push(
    new ethereum.EventParam("to", ethereum.Value.fromAddress(to)),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "tokenId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  return event;
}

export function uriUpdated(id: i32, uri: string): URIUpdated {
  const event = changetype<URIUpdated>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam(
      "agentId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("newURI", ethereum.Value.fromString(uri)),
  );
  event.parameters.push(
    new ethereum.EventParam("updatedBy", ethereum.Value.fromAddress(OWNER)),
  );
  return event;
}

export function uwuTransfer(
  from: Address,
  to: Address,
  value: string,
): UwuTransfer {
  const event = changetype<UwuTransfer>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam("from", ethereum.Value.fromAddress(from)),
  );
  event.parameters.push(
    new ethereum.EventParam("to", ethereum.Value.fromAddress(to)),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "value",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromString(value)),
    ),
  );
  return event;
}

export function feedback(id: i32, index: i32 = 1): NewFeedback {
  const event = changetype<NewFeedback>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam(
      "agentId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("clientAddress", ethereum.Value.fromAddress(WALLET_TWO)),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "feedbackIndex",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(index)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "value",
      ethereum.Value.fromSignedBigInt(BigInt.fromI32(-15)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "valueDecimals",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(1)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("indexedTag1", ethereum.Value.fromBytes(HASH)),
  );
  event.parameters.push(
    new ethereum.EventParam("tag1", ethereum.Value.fromString("reliability")),
  );
  event.parameters.push(
    new ethereum.EventParam("tag2", ethereum.Value.fromString("xmtp")),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "endpoint",
      ethereum.Value.fromString("xmtp://fixture"),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("feedbackURI", ethereum.Value.fromString("ipfs://fixture")),
  );
  event.parameters.push(
    new ethereum.EventParam("feedbackHash", ethereum.Value.fromFixedBytes(HASH)),
  );
  return event;
}

export function revoked(id: i32, index: i32 = 1): FeedbackRevoked {
  const event = changetype<FeedbackRevoked>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam(
      "agentId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("clientAddress", ethereum.Value.fromAddress(WALLET_TWO)),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "feedbackIndex",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(index)),
    ),
  );
  return event;
}

export function response(id: i32, index: i32 = 1): ResponseAppended {
  const event = changetype<ResponseAppended>(newMockEvent());
  event.parameters = new Array<ethereum.EventParam>();
  event.parameters.push(
    new ethereum.EventParam(
      "agentId",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(id)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("clientAddress", ethereum.Value.fromAddress(WALLET_TWO)),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "feedbackIndex",
      ethereum.Value.fromUnsignedBigInt(BigInt.fromI32(index)),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("responder", ethereum.Value.fromAddress(OWNER)),
  );
  event.parameters.push(
    new ethereum.EventParam(
      "responseURI",
      ethereum.Value.fromString("ipfs://response"),
    ),
  );
  event.parameters.push(
    new ethereum.EventParam("responseHash", ethereum.Value.fromFixedBytes(HASH)),
  );
  return event;
}
