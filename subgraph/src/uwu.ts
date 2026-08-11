import { Transfer } from "../generated/UWU/UWU";
import { isZeroAddress } from "./constants";
import { getOrCreateWallet } from "./models";

export function handleTransfer(event: Transfer): void {
  const fromIsZero = isZeroAddress(event.params.from);
  const toIsZero = isZeroAddress(event.params.to);

  if (!fromIsZero && event.params.from.equals(event.params.to)) {
    const wallet = getOrCreateWallet(event.params.from, event.block);
    wallet.updatedBlock = event.block.number;
    wallet.updatedTimestamp = event.block.timestamp;
    wallet.save();
    return;
  }

  if (!fromIsZero) {
    const sender = getOrCreateWallet(event.params.from, event.block);
    assert(
      sender.rawBalance.ge(event.params.value),
      "UWU balance underflow: verify address and start block",
    );
    sender.rawBalance = sender.rawBalance.minus(event.params.value);
    sender.updatedBlock = event.block.number;
    sender.updatedTimestamp = event.block.timestamp;
    sender.save();
  }

  if (!toIsZero) {
    const recipient = getOrCreateWallet(event.params.to, event.block);
    recipient.rawBalance = recipient.rawBalance.plus(event.params.value);
    recipient.updatedBlock = event.block.number;
    recipient.updatedTimestamp = event.block.timestamp;
    recipient.save();
  }
}
