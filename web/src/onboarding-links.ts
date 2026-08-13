import { getAddress } from "ethers";
import type { XmtpEnvironment } from "./config";

const ZERO = "0x0000000000000000000000000000000000000000";

export interface OnboardingLink {
  tentacle?: string;
  referrer?: string;
}

export function parseOnboardingLink(hash: string): OnboardingLink {
  const params = new URLSearchParams(hash.startsWith("#") ? hash.slice(1) : hash);
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

export function recruitmentUrl(origin: string, tentacle: string, referrer: string): string {
  const target = getAddress(tentacle).toLowerCase();
  const recruiter = getAddress(referrer).toLowerCase();
  if (target === ZERO || recruiter === ZERO) throw new Error("Recruitment addresses must be nonzero");
  return `${new URL(origin).origin}/#t=${target}&r=${recruiter}`;
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
