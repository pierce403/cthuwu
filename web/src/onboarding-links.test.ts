import { describe, expect, it } from "vitest";
import { parseOnboardingLink, pinReferrer, recruitmentUrl } from "./onboarding-links";

const tentacle = "0x1111111111111111111111111111111111111111";
const referrer = "0x2222222222222222222222222222222222222222";

describe("onboarding links", () => {
  it("parses canonical t and r address parameters", () => {
    expect(parseOnboardingLink(`#t=${tentacle}&r=${referrer}`)).toEqual({ tentacle, referrer });
  });

  it("rejects malformed, zero, and duplicated authority parameters", () => {
    expect(() => parseOnboardingLink("#t=nope")).toThrow(/nonzero Ethereum/u);
    expect(() => parseOnboardingLink("#r=0x0000000000000000000000000000000000000000")).toThrow(/nonzero Ethereum/u);
    expect(() => parseOnboardingLink(`#t=${tentacle}&t=${referrer}`)).toThrow(/appear once/u);
  });

  it("does not read ordinary HTTP query parameters", () => {
    expect(parseOnboardingLink("")).toEqual({});
  });

  it("pins the first referral for an acolyte and refuses later replacement", () => {
    const values = new Map<string, string>();
    const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
    expect(pinReferrer("production", tentacle, referrer, storage)).toBe(referrer);
    expect(pinReferrer("production", tentacle, "0x3333333333333333333333333333333333333333", storage)).toBe(referrer);
  });

  it("builds a browser-only social referral URL", () => {
    expect(recruitmentUrl("https://cthuwu.app/anything?ignored=yes", tentacle, referrer)).toBe(
      `https://cthuwu.app/#t=${tentacle}&r=${referrer}`,
    );
  });
});
