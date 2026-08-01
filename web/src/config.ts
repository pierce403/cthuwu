import { ZeroAddress, getAddress } from "ethers";

export type XmtpEnvironment = "dev" | "production" | "local";

export interface AppConfig {
  environment: XmtpEnvironment;
  botAddress: string;
}

export function parseEnvironment(value: string | undefined): XmtpEnvironment {
  if (value === undefined || value === "") return "dev";
  if (value === "dev" || value === "production" || value === "local") return value;
  throw new Error("VITE_XMTP_ENV must be dev, production, or local");
}

export function parseConfig(environment: XmtpEnvironment, botAddress: string | undefined): AppConfig {
  const candidate = botAddress?.trim();
  if (!candidate) throw new Error("Cthuwu's XMTP address is not configured");
  try {
    const address = getAddress(candidate).toLowerCase();
    if (address === ZeroAddress) throw new Error("zero address");
    return { environment, botAddress: address };
  } catch {
    // It may be an ENS name instead of a hexadecimal address.
  }
  if (/^[a-z0-9-]+(?:\.[a-z0-9-]+)*\.eth$/i.test(candidate)) {
    return { environment, botAddress: candidate.toLowerCase() };
  }
  throw new Error("Cthuwu's XMTP address must be an Ethereum address or ENS name");
}
