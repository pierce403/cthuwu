import { describe, expect, it } from "vitest";
import { compareRawBalances, formatLevel, formatWholeUwu, tentacleLevel } from "./level";

describe("Tentacle Level", () => {
  it.each([
    ["0", undefined, "UNFUNDED"],
    ["100000000000000000", -1, "-1.00"],
    ["1000000000000000000", 0, "0.00"],
    ["10000000000000000000", 1, "1.00"],
    ["100000000000000000000", 2, "2.00"],
    ["1000000000000000000000", 3, "3.00"],
    ["1000000000000000000000000", 6, "6.00"],
    ["1000000000000000000000000000", 9, "9.00"],
    ["100000000000000000000000000000", 11, "11.00"],
  ])("calculates %s raw UWU", (raw, expected, rendered) => {
    expect(tentacleLevel(raw)).toBe(expected);
    expect(formatLevel(raw)).toBe(rendered);
  });

  it("retains fractional precision without converting the uint256 to Number", () => {
    expect(tentacleLevel("2500000000000000000")).toBeCloseTo(Math.log10(2.5), 12);
    expect(formatLevel("2500000000000000000")).toBe("0.40");
    expect(tentacleLevel(((1n << 256n) - 1n).toString())).toBeCloseTo(59.06367888997919, 12);
  });

  it("formats exact human-denominated balances and sorts by BigInt", () => {
    expect(formatWholeUwu("1000000000000000010")).toBe("1.00000000000000001");
    expect(formatWholeUwu("1000000000000000000000")).toBe("1,000");
    const values = ["9", "100000000000000000000", "10", "0"];
    expect(values.sort(compareRawBalances)).toEqual(["100000000000000000000", "10", "9", "0"]);
  });

  it("rejects noncanonical and out-of-range values", () => {
    expect(() => tentacleLevel("01")).toThrow();
    expect(() => tentacleLevel("-1")).toThrow();
    expect(() => tentacleLevel((1n << 256n).toString())).toThrow();
  });
});
