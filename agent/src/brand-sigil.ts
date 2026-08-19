import { keccak256, toHex } from "viem";

export interface BrandSigilOptions {
  acolyte: string;
  controllerAgentId: string;
  acolyteName?: string;
}

export function generateBrandSigilSvg(options: BrandSigilOptions): string {
  const seedHex = keccak256(
    toHex(`cthuwu-brand-sigil-v1:${options.acolyte.toLowerCase()}:${options.controllerAgentId}`),
  );
  const bytes = Buffer.from(seedHex.slice(2), "hex");

  // Deterministic geometry
  const r1 = 118 + (bytes[0]! % 20);
  const r2 = 72 + (bytes[1]! % 16);
  const rot = Math.round((bytes[2]! * 360) / 256);
  const pts = 5 + (bytes[3]! % 4);

  const starPts: string[] = [];
  for (let i = 0; i < pts * 2; i++) {
    const angle = (i * Math.PI) / pts;
    const r = i % 2 === 0 ? r1 : r2;
    starPts.push(`${i === 0 ? "M" : "L"}${Math.round(256 + r * Math.cos(angle))} ${Math.round(256 + r * Math.sin(angle))}`);
  }
  starPts.push("Z");
  const star = starPts.join(" ");

  const d1 = 256 - r1;
  const d2 = 256 + r1;
  const runes = `M${d1 - 12} 256h16M${d1 - 4} 248v16M${d2 - 4} 256h16M${d2 + 4} 248v16M256 ${d1 - 12}v16M248 ${d1 - 4}h16M256 ${d2 - 4}v16M248 ${d2 + 4}h16`;

  const rawName = options.acolyteName ?? `Acolyte ${options.acolyte.slice(0, 6)}…${options.acolyte.slice(-4)}`;
  const label = rawName
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><defs><radialGradient id="f"><stop offset="0" stop-color="#140202"/><stop offset=".5" stop-color="#5a1810"/><stop offset="1" stop-color="#a85838"/></radialGradient><radialGradient id="g"><stop offset="0" stop-color="#f90"/><stop offset="1" stop-color="#300" stop-opacity="0"/></radialGradient><filter id="b"><feDropShadow dx="0" dy="0" stdDeviation="4" flood-color="#f60"/></filter></defs><rect width="512" height="512" rx="64" fill="url(#f)"/><circle cx="256" cy="256" r="200" fill="url(#g)" stroke="#260404" stroke-width="20"/><circle cx="256" cy="256" r="165" fill="none" stroke="#f40" stroke-width="4" stroke-dasharray="12,8" filter="url(#b)"/><g transform="rotate(${rot} 256 256)" filter="url(#b)"><path d="${star}" fill="none" stroke="#fa0" stroke-width="4"/><circle cx="256" cy="256" r="40" fill="#0d0000" stroke="#f60" stroke-width="4"/><path d="M236 256Q256 232 276 256Q256 280 236 256Z" fill="#fc0"/><circle cx="256" cy="256" r="5" fill="#140202"/></g><path d="${runes}" stroke="#f60" stroke-width="3" stroke-linecap="round" filter="url(#b)"/><text x="256" y="475" text-anchor="middle" font-family="system-ui,sans-serif" font-weight="800" font-size="17" fill="#fd9">${label}</text></svg>`;
}

export function generateBrandSigilDataUri(options: BrandSigilOptions): string {
  const svg = generateBrandSigilSvg(options);
  return `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
}
