import EthereumProvider from "@walletconnect/ethereum-provider";
import { IdentifierKind, type Signer } from "@xmtp/browser-sdk";
import { BrowserProvider, getAddress, getBytes, type Eip1193Provider } from "ethers";
import type { ExternalIdentity, ExternalWalletConnector } from "./identity";

// Reown project identifiers are public application identifiers, not secrets. This one is shared
// with converge.cv and may be replaced at build time without changing the checked-in fallback.
export const WALLETCONNECT_PROJECT_ID =
  import.meta.env.VITE_WALLETCONNECT_PROJECT_ID || "de49d3fcfa0a614710c571a3484a4d0f";

const SUPPORTED_CHAINS = [1, 8453, 84532] as const;

interface InjectedProvider extends Eip1193Provider {
  request(args: { method: string; params?: readonly unknown[] | object }): Promise<unknown>;
}

export interface ConnectedExternalWallet {
  address: string;
  chainId: number;
  connector: ExternalWalletConnector;
  signerType: "EOA" | "SCW";
}

let walletConnectProvider: Promise<EthereumProvider> | undefined;

export async function connectExternalWallet(
  kind: ExternalWalletConnector,
): Promise<ConnectedExternalWallet> {
  const provider = kind === "walletConnect"
    ? await connectedWalletConnectProvider()
    : requireInjectedProvider();
  const accounts = (kind === "walletConnect"
    ? await provider.request<string[]>({ method: "eth_accounts" })
    : await provider.request({ method: "eth_requestAccounts" })) as string[];
  return inspectConnection(provider, kind, accounts);
}

export async function disconnectExternalWallet(): Promise<void> {
  if (!walletConnectProvider) return;
  const provider = await walletConnectProvider;
  if (provider.session) await provider.disconnect();
}

export async function createExternalSigner(identity: ExternalIdentity): Promise<Signer> {
  const provider = await restoredProvider(identity.connector);
  const accounts = await provider.request({ method: "eth_accounts" }) as string[];
  if (!accounts.some((account) => canonicalAddress(account) === identity.address)) {
    throw new Error(
      `Reconnect ${connectorLabel(identity.connector)} in identity settings to use ${shortAddress(identity.address)}.`,
    );
  }
  if (identity.signerType === "SCW") {
    const activeChainId = parseChainId(await provider.request({ method: "eth_chainId" }));
    if (activeChainId !== identity.chainId) {
      throw new Error(`Reconnect the smart account on chain ${identity.chainId} before opening XMTP`);
    }
  }
  const browserProvider = new BrowserProvider(provider);
  const signerCore = {
    getIdentifier: () => ({
      identifier: identity.address,
      identifierKind: IdentifierKind.Ethereum,
    }),
    signMessage: async (message: string) => {
      const currentAccounts = await provider.request({ method: "eth_accounts" }) as string[];
      if (!currentAccounts.some((account) => canonicalAddress(account) === identity.address)) {
        throw new Error("The connected wallet account changed; reconnect the saved Acolyte wallet");
      }
      const walletSigner = await browserProvider.getSigner(identity.address);
      return getBytes(await walletSigner.signMessage(message));
    },
  };
  return identity.signerType === "SCW"
    ? { ...signerCore, type: "SCW", getChainId: () => BigInt(identity.chainId) }
    : { ...signerCore, type: "EOA" };
}

async function connectedWalletConnectProvider(): Promise<EthereumProvider> {
  const provider = await getWalletConnectProvider();
  if (!provider.session) await provider.connect();
  return provider;
}

async function restoredProvider(connector: ExternalWalletConnector): Promise<InjectedProvider> {
  if (connector === "injected") return requireInjectedProvider();
  const provider = await getWalletConnectProvider();
  if (!provider.session) {
    throw new Error("Reconnect WalletConnect in identity settings before opening XMTP");
  }
  return provider;
}

function getWalletConnectProvider(): Promise<EthereumProvider> {
  walletConnectProvider ??= EthereumProvider.init({
    projectId: WALLETCONNECT_PROJECT_ID,
    chains: [8453],
    optionalChains: [1, 84532],
    showQrModal: true,
    metadata: {
      name: "Cthuwu",
      description: "Private XMTP chat with a Cthuwu Tentacle",
      url: "https://cthuwu.app",
      icons: ["https://cthuwu.app/icons/icon-192.png"],
    },
    qrModalOptions: { themeMode: "dark" },
  });
  return walletConnectProvider;
}

function requireInjectedProvider(): InjectedProvider {
  const provider = (globalThis as typeof globalThis & { ethereum?: InjectedProvider }).ethereum;
  if (!provider) throw new Error("No browser wallet was found. Install or unlock one, then retry.");
  return provider;
}

async function inspectConnection(
  provider: InjectedProvider,
  connector: ExternalWalletConnector,
  accounts: readonly string[],
): Promise<ConnectedExternalWallet> {
  const address = canonicalAddress(accounts[0]);
  const chainId = parseChainId(await provider.request({ method: "eth_chainId" }));
  if (!SUPPORTED_CHAINS.includes(chainId as (typeof SUPPORTED_CHAINS)[number])) {
    throw new Error("Connect the wallet on Ethereum, Base, or Base Sepolia, then retry.");
  }
  let bytecode: string;
  try {
    bytecode = await new BrowserProvider(provider).getCode(address);
  } catch {
    throw new Error("Could not inspect the connected account type. Check its network and retry.");
  }
  return {
    address,
    chainId,
    connector,
    signerType: bytecode !== "0x" ? "SCW" : "EOA",
  };
}

function parseChainId(value: unknown): number {
  if (typeof value !== "string" || !/^0x[0-9a-f]+$/iu.test(value)) {
    throw new Error("The wallet returned an invalid chain ID");
  }
  const chainId = Number.parseInt(value.slice(2), 16);
  if (!Number.isSafeInteger(chainId) || chainId <= 0) throw new Error("The wallet returned an invalid chain ID");
  return chainId;
}

function canonicalAddress(address: string | undefined): string {
  if (!address) throw new Error("The wallet did not return an account");
  const normalized = getAddress(address).toLowerCase();
  if (/^0x0{40}$/u.test(normalized)) throw new Error("The wallet returned the zero address");
  return normalized;
}

function connectorLabel(connector: ExternalWalletConnector): string {
  return connector === "walletConnect" ? "WalletConnect" : "the browser wallet";
}

function shortAddress(address: string): string {
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}
