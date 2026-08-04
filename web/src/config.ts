export type XmtpEnvironment = "dev" | "production" | "local";

// Temporary bootstrap discovery until intro Tentacles can register through the planned Base
// registry. This is public routing metadata, never an identity secret.
export const INTRO_TENTACLE_ADDRESS = "0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db";

export interface AppConfig {
  environment: XmtpEnvironment;
  botAddress: string;
}

export function parseEnvironment(value: string | undefined): XmtpEnvironment {
  if (value === undefined || value === "") return "dev";
  if (value === "dev" || value === "production" || value === "local") return value;
  throw new Error("VITE_XMTP_ENV must be dev, production, or local");
}

export function parseConfig(environment: XmtpEnvironment): AppConfig {
  return { environment, botAddress: INTRO_TENTACLE_ADDRESS };
}
