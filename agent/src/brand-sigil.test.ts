import { describe, expect, it } from "vitest";
import { generateBrandSigilDataUri, generateBrandSigilSvg } from "./brand-sigil.js";

describe("brand sigil generator", () => {
  const acolyte = "0x1234567890123456789012345678901234567890";
  const controllerAgentId = "42";
  const acolyteName = "Eldritch Disciple";

  it("generates a valid SVG string with burned flesh and eldritch sigil elements", () => {
    const svg = generateBrandSigilSvg({ acolyte, controllerAgentId, acolyteName });
    expect(svg).toContain("<svg");
    expect(svg).toContain("</svg>");
    expect(svg).toContain(acolyteName);
    expect(svg).toContain("radialGradient");
    expect(svg).toContain("#f90"); // ember color
  });

  it("produces an on-chain data URI strictly within MAX_AVATAR_URI_BYTES (2,048 bytes)", () => {
    const uri = generateBrandSigilDataUri({ acolyte, controllerAgentId, acolyteName });
    expect(uri.startsWith("data:image/svg+xml;")).toBe(true);
    expect(Buffer.byteLength(uri, "utf8")).toBeLessThanOrEqual(2048);
  });

  it("generates distinct sigils for different acolytes", () => {
    const svg1 = generateBrandSigilSvg({ acolyte: "0x1111111111111111111111111111111111111111", controllerAgentId: "42" });
    const svg2 = generateBrandSigilSvg({ acolyte: "0x2222222222222222222222222222222222222222", controllerAgentId: "42" });
    expect(svg1).not.toBe(svg2);
  });

  it("escapes special characters in the acolyte name", () => {
    const svg = generateBrandSigilSvg({
      acolyte,
      controllerAgentId,
      acolyteName: 'Acolyte <Test> & "Quotes"',
    });
    expect(svg).toContain("Acolyte &lt;Test&gt; &amp; &quot;Quotes&quot;");
    expect(svg).not.toContain("<Test>");
  });
});
