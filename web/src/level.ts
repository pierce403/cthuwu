import { UWU_DECIMALS } from "./leaderboard-types";

const RAW_BALANCE = /^(0|[1-9][0-9]{0,77})$/u;
const UINT256_MAX = (1n << 256n) - 1n;
const MANTISSA_DIGITS = 15;

export function parseRawBalance(value: string): bigint {
  if (!RAW_BALANCE.test(value)) throw new Error("invalid raw UWU balance");
  const parsed = BigInt(value);
  if (parsed > UINT256_MAX) throw new Error("raw UWU balance exceeds uint256");
  return parsed;
}

/**
 * Returns log10(rawBalance) - 18 without ever converting the uint256 balance to Number.
 * The Number conversion is limited to a normalized 15-significant-digit mantissa in [1, 10),
 * which keeps substantially more precision than the two decimals shown by the interface.
 */
export function tentacleLevel(rawBalance: string): number | undefined {
  const raw = parseRawBalance(rawBalance);
  if (raw === 0n) return undefined;
  const digits = raw.toString();
  const significant = digits.slice(0, MANTISSA_DIGITS).padEnd(MANTISSA_DIGITS, "0");
  const mantissa = Number(`${significant[0]}.${significant.slice(1)}`);
  return digits.length - 1 - UWU_DECIMALS + Math.log10(mantissa);
}

export function formatLevel(rawBalance: string): string {
  const level = tentacleLevel(rawBalance);
  if (level === undefined) return "UNFUNDED";
  const rendered = level.toFixed(2);
  return rendered === "-0.00" ? "0.00" : rendered;
}

export function formatWholeUwu(rawBalance: string): string {
  const digits = parseRawBalance(rawBalance).toString().padStart(UWU_DECIMALS + 1, "0");
  const whole = digits.slice(0, -UWU_DECIMALS).replace(/\B(?=(\d{3})+(?!\d))/gu, ",");
  const fraction = digits.slice(-UWU_DECIMALS).replace(/0+$/u, "");
  return fraction.length > 0 ? `${whole}.${fraction}` : whole;
}

export function compareRawBalances(left: string, right: string): number {
  const a = parseRawBalance(left);
  const b = parseRawBalance(right);
  return a === b ? 0 : a > b ? -1 : 1;
}
