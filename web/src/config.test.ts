import { describe, expect, it } from "vitest";
import { parseConfig, parseEnvironment } from "./config";

describe("configuration", () => {
  it("defaults only an absent environment to dev", () => {
    expect(parseEnvironment(undefined)).toBe("dev");
    expect(() => parseEnvironment("staging")).toThrow("must be dev");
  });

  it("accepts an address or normalized ENS name", () => {
    expect(parseConfig("dev", "CTHULHUBOT.ETH").botAddress).toBe("cthulhubot.eth");
    expect(
      parseConfig("production", "0x0000000000000000000000000000000000000001").botAddress,
    ).toBe("0x0000000000000000000000000000000000000001");
    expect(() =>
      parseConfig("dev", "0x0000000000000000000000000000000000000000"),
    ).toThrow("Ethereum address");
    expect(() => parseConfig("dev", "not a destination")).toThrow("Ethereum address");
  });
});
