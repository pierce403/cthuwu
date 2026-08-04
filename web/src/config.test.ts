import { describe, expect, it } from "vitest";
import { INTRO_TENTACLE_ADDRESS, parseConfig, parseEnvironment } from "./config";

describe("configuration", () => {
  it("defaults only an absent environment to dev", () => {
    expect(parseEnvironment(undefined)).toBe("dev");
    expect(() => parseEnvironment("staging")).toThrow("must be dev");
  });

  it("always selects the hard-coded intro Tentacle", () => {
    expect(INTRO_TENTACLE_ADDRESS).toBe("0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db");
    expect(parseConfig("dev")).toEqual({
      environment: "dev",
      botAddress: INTRO_TENTACLE_ADDRESS,
    });
    expect(parseConfig("production").botAddress).toBe(INTRO_TENTACLE_ADDRESS);
  });
});
