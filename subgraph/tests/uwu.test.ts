import { afterEach, assert, clearStore, describe, test } from "matchstick-as";
import { handleTransfer } from "../src/uwu";
import { WALLET, WALLET_TWO, ZERO, uwuTransfer } from "./helpers";

afterEach(() => clearStore());

describe("exact UWU balances", () => {
  test("handles mint, ordinary transfer, burn, and self-transfer", () => {
    handleTransfer(uwuTransfer(ZERO, WALLET, "100000000000000000000"));
    assert.fieldEquals(
      "Wallet",
      WALLET.toHexString(),
      "rawBalance",
      "100000000000000000000",
    );

    handleTransfer(uwuTransfer(WALLET, WALLET_TWO, "25000000000000000000"));
    assert.fieldEquals(
      "Wallet",
      WALLET.toHexString(),
      "rawBalance",
      "75000000000000000000",
    );
    assert.fieldEquals(
      "Wallet",
      WALLET_TWO.toHexString(),
      "rawBalance",
      "25000000000000000000",
    );

    handleTransfer(uwuTransfer(WALLET_TWO, ZERO, "5000000000000000000"));
    assert.fieldEquals(
      "Wallet",
      WALLET_TWO.toHexString(),
      "rawBalance",
      "20000000000000000000",
    );

    handleTransfer(uwuTransfer(WALLET, WALLET, "1000000000000000000"));
    assert.fieldEquals(
      "Wallet",
      WALLET.toHexString(),
      "rawBalance",
      "75000000000000000000",
    );
  });
});
