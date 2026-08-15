import { afterEach, describe, expect, it, vi } from "vitest";
import { connectExternalWallet, WALLETCONNECT_PROJECT_ID } from "./wallet-connector";

const ADDRESS = "0x2222222222222222222222222222222222222222";

describe("external wallet connector", () => {
  afterEach(() => {
    Reflect.deleteProperty(globalThis, "ethereum");
  });

  it("uses the public Reown project identifier shared with Converge", () => {
    expect(WALLETCONNECT_PROJECT_ID).toBe("de49d3fcfa0a614710c571a3484a4d0f");
  });

  it("connects an injected EOA without reading a private key", async () => {
    const request = vi.fn(async ({ method }: { method: string }) => {
      if (method === "eth_requestAccounts" || method === "eth_accounts") return [ADDRESS];
      if (method === "eth_chainId") return "0x2105";
      if (method === "eth_getCode") return "0x";
      throw new Error(`unexpected ${method}`);
    });
    Object.defineProperty(globalThis, "ethereum", { configurable: true, value: { request } });

    await expect(connectExternalWallet("injected")).resolves.toEqual({
      address: ADDRESS,
      chainId: 8453,
      connector: "injected",
      signerType: "EOA",
    });
    expect(request.mock.calls.map(([argument]) => argument.method)).not.toContain("eth_privateKey");
  });

  it("rejects missing browser wallets and unsupported chains", async () => {
    await expect(connectExternalWallet("injected")).rejects.toThrow("No browser wallet");
    Object.defineProperty(globalThis, "ethereum", {
      configurable: true,
      value: { request: vi.fn(async ({ method }: { method: string }) =>
        method === "eth_chainId" ? "0x89" : [ADDRESS]) },
    });
    await expect(connectExternalWallet("injected")).rejects.toThrow("Ethereum, Base, or Base Sepolia");
  });
});
