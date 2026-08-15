import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function routeHtml(route: "tentacles" | "acolytes"): Document {
  const html = readFileSync(resolve(process.cwd(), route, "index.html"), "utf8");
  return new DOMParser().parseFromString(html, "text/html");
}

describe("public directory routes", () => {
  it.each(["tentacles", "acolytes"] as const)("builds %s as an accessible direct entry", (route) => {
    const document = routeHtml(route);
    expect(document.querySelector(`link[rel="canonical"]`)?.getAttribute("href")).toBe(
      `https://cthuwu.app/${route}/`,
    );
    expect(document.querySelector('a[href="/"]')).not.toBeNull();
    expect(document.querySelector('a[href="/tentacles/"]')).not.toBeNull();
    expect(document.querySelector('a[href="/acolytes/"]')).not.toBeNull();
    expect(document.querySelector('a[href="/operator/"]')).not.toBeNull();
    expect(document.querySelector('a[href="https://github.com/pierce403/cthuwu"]')?.textContent).toContain(
      "GitHub",
    );
    const csp = document.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute("content");
    expect(csp).toContain("img-src 'self' data:");
    expect(csp).not.toMatch(/img-src[^;]*https:/u);
  });

  it("keeps the leaderboard only on the Tentacles route", () => {
    expect(routeHtml("tentacles").querySelector("#leaderboard-list")?.getAttribute("role")).toBe("list");
    expect(routeHtml("acolytes").querySelector("#leaderboard-list")).toBeNull();
  });

  it("exposes explicit Acolyte loading, empty, and error states", () => {
    const document = routeHtml("acolytes");
    expect(document.querySelector("#acolyte-state")?.getAttribute("role")).toBe("status");
    expect(document.querySelector("#acolyte-list")?.getAttribute("role")).toBe("list");
    expect(document.querySelector("#acolyte-empty")?.textContent).toContain("No Acolyte Branding NFTs");
    expect(document.querySelector("#acolyte-error")?.getAttribute("role")).toBe("alert");
  });

  it("ships a separate direct-only operator entry without claiming browser authority", () => {
    const html = readFileSync(resolve(process.cwd(), "operator", "index.html"), "utf8");
    const document = new DOMParser().parseFromString(html, "text/html");
    expect(document.querySelector('link[rel="canonical"]')?.getAttribute("href"))
      .toBe("https://cthuwu.app/operator/");
    expect(document.querySelector("#operator-target-form")).not.toBeNull();
    expect(document.querySelector("#messages")?.getAttribute("role")).toBe("log");
    expect(document.querySelector('[role="tablist"]')?.textContent).toContain("Direct operator DM");
    expect(document.querySelector("#operator-title")?.parentElement?.textContent).toContain(
      "cannot grant a role",
    );
    expect(document.body.textContent).toContain("Remote-code-execution boundary");
    expect(document.body.textContent).toContain("same Acolyte EOA and XMTP inbox");
    expect(document.body.textContent).toContain("only the public EOA");
    expect(document.querySelector("#operator-launch-command")).not.toBeNull();
    expect(document.querySelector('script[src="/src/operator.ts"]')).not.toBeNull();
    const csp = document.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute("content");
    expect(csp).toContain("connect-src 'self' https: wss:");
  });
});
