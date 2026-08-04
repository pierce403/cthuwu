export type XmtpEnvironment = "dev" | "production" | "local";

export const XMTP_ENVIRONMENT: XmtpEnvironment = "production";

// Temporary bootstrap discovery until intro Tentacles can register through the planned Base
// registry. This is public routing metadata, never an identity secret.
export const INTRO_TENTACLE_ADDRESS = "0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db";

export interface AppConfig {
  environment: XmtpEnvironment;
  botAddress: string;
}

export function parseConfig(): AppConfig {
  return { environment: XMTP_ENVIRONMENT, botAddress: INTRO_TENTACLE_ADDRESS };
}
