import { getAddress } from "ethers";
import type { XmtpEnvironment } from "./config";

const ZERO = "0x0000000000000000000000000000000000000000";

export interface OnboardingLink {
  tentacle?: string;
  referrer?: string;
}

export function parseOnboardingLink(search: string): OnboardingLink {
  const params = new URLSearchParams(search);
  return {
    ...(params.has("t") ? { tentacle: addressParam(params, "t") } : {}),
    ...(params.has("r") ? { referrer: addressParam(params, "r") } : {}),
  };
}

export function pinReferrer(
  environment: XmtpEnvironment,
  acolyte: string,
  offered: string | undefined,
  storage: Pick<Storage, "getItem" | "setItem"> = localStorage,
): string | undefined {
  const key = `cthuwu.referrer.v1:${environment}:${acolyte.toLowerCase()}`;
  const existing = storage.getItem(key) ?? undefined;
  if (existing) return existing;
  if (offered) storage.setItem(key, offered);
  return offered;
}

function addressParam(params: URLSearchParams, name: string): string {
  if (params.getAll(name).length !== 1) throw new Error(`The ${name} link parameter must appear once`);
  try {
    const value = getAddress(params.get(name) ?? "").toLowerCase();
    if (value === ZERO) throw new Error("zero");
    return value;
  } catch {
    throw new Error(`The ${name} link parameter must be a nonzero Ethereum address`);
  }
}
