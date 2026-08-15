import { describe, expect, it } from "vitest";
import {
  ACOLYTE_NAME_SCHEME,
  ACOLYTE_NAME_TRAIT,
  acolyteName,
  acolyteNameSpaceSize,
  acolyteNameTableFingerprint,
  nftAcolyteName,
} from "./acolyte-name";

describe("Acolyte names", () => {
  it("derives stable stuffy compound surnames from random EOA identities", () => {
    expect(acolyteName("0x0000000000000000000000000000000000000001")).toBe(
      "Broughton-Arbuthnot of Marshborough",
    );
    expect(acolyteName("0x1111111111111111111111111111111111111111")).toBe(
      "Ainsworth-Clavering of Ambercroft",
    );
    expect(acolyteName("0x0000000000000000000000000000000000000001")).toMatch(
      /^[A-Z][A-Za-z]+-[A-Z][A-Za-z]+ of [A-Z][A-Za-z]+$/u,
    );
  });

  it("freezes a large versioned name table instead of silently renaming identities", () => {
    expect(ACOLYTE_NAME_SCHEME).toBe("acolyte-v1");
    expect(acolyteNameSpaceSize()).toBe(16_777_216);
    expect(acolyteNameTableFingerprint()).toBe(
      "0x3b88f4b2c9942c63c6234de53389f745b1766331881d2d03026fa0b6c3439d23",
    );
  });

  it("uses only the exact reserved NFT trait and treats the value as owner metadata", () => {
    expect(nftAcolyteName([{ traitType: ACOLYTE_NAME_TRAIT, value: " Pemberton-Smythe " }]))
      .toBe("Pemberton-Smythe");
    expect(nftAcolyteName([{ traitType: "acolyte name", value: "spoof" }])).toBeUndefined();
    expect(nftAcolyteName([{ traitType: ACOLYTE_NAME_TRAIT, value: "" }])).toBeUndefined();
  });
});
