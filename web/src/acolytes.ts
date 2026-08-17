import "./style.css";

import {
  fetchAcolyteCatalog,
  type AcolyteCatalogItem,
  type AcolyteCatalogSnapshot,
} from "./acolyte-data";
import { parseConfig } from "./config";
import { acolyteName, nftAcolyteName } from "./acolyte-name";

const BASE_EXPLORER = "https://basescan.org";
const MAX_RENDERED_ACOLYTES = 5_000;
const ADDRESS = /^0x[0-9a-fA-F]{40}$/u;
const TRANSACTION_HASH = /^0x[0-9a-fA-F]{64}$/u;

function requireElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing required Acolyte catalog element: ${id}`);
  return element as T;
}

const rootElement = requireElement<HTMLElement>("acolytes");
const stateElement = requireElement<HTMLElement>("acolyte-state");
const sourceElement = requireElement<HTMLElement>("acolyte-source");
const contractElement = requireElement<HTMLAnchorElement>("acolyte-contract");
const refreshElement = requireElement<HTMLButtonElement>("acolyte-refresh");
const summaryElement = requireElement<HTMLElement>("acolyte-summary");
const listElement = requireElement<HTMLElement>("acolyte-list");
const emptyElement = requireElement<HTMLElement>("acolyte-empty");
const errorElement = requireElement<HTMLElement>("acolyte-error");

let activeRequest: AbortController | undefined;
let disposed = false;

refreshElement.addEventListener("click", () => void refresh());
window.addEventListener("online", () => void refresh());
window.addEventListener("pagehide", (event) => {
  if (event.persisted) return;
  disposed = true;
  activeRequest?.abort();
});

void refresh();

async function refresh(): Promise<void> {
  if (disposed) return;
  activeRequest?.abort();
  const request = new AbortController();
  activeRequest = request;
  setLoading();

  try {
    const config = parseConfig();
    if (!config.brandingContract) throw new Error("Acolyte Branding contract is not configured");
    const result = await fetchAcolyteCatalog({
      endpoint: config.baseRpcEndpoint,
      signal: request.signal,
    });
    if (disposed || request.signal.aborted) return;
    renderCatalog(result);
  } catch (error) {
    if (disposed || request.signal.aborted || isAbortError(error)) return;
    console.error("The public Acolyte catalog could not be loaded", error);
    renderError(navigator.onLine === false ? "OFFLINE" : "UNAVAILABLE");
  } finally {
    if (activeRequest === request) activeRequest = undefined;
  }
}

function setLoading(): void {
  rootElement.dataset.state = "refreshing";
  rootElement.setAttribute("aria-busy", "true");
  listElement.setAttribute("aria-busy", "true");
  stateElement.textContent = "REFRESHING";
  sourceElement.textContent = "Reading a fixed Base block…";
  summaryElement.textContent = "Looking for minted Acolyte NFTs…";
  refreshElement.disabled = true;
  emptyElement.hidden = true;
  errorElement.hidden = true;
}

function renderCatalog(catalog: AcolyteCatalogSnapshot): void {
  const items = catalog.items.slice(0, MAX_RENDERED_ACOLYTES);
  const truncated = catalog.items.length > MAX_RENDERED_ACOLYTES;
  rootElement.dataset.state = items.length === 0 ? "empty" : "current";
  rootElement.setAttribute("aria-busy", "false");
  listElement.setAttribute("aria-busy", "false");
  stateElement.textContent = items.length === 0 ? "EMPTY" : "CURRENT";
  sourceElement.textContent = `Base 8453 · block ${catalog.sourceBlockNumber}`;
  summaryElement.textContent = `${catalog.items.length} minted Acolyte NFT${catalog.items.length === 1 ? "" : "s"}${
    truncated ? ` · showing first ${MAX_RENDERED_ACOLYTES}` : ""
  }`;
  refreshElement.disabled = false;
  errorElement.hidden = true;
  emptyElement.hidden = items.length > 0;
  listElement.replaceChildren(...items.map(renderAcolyte));
  if (ADDRESS.test(catalog.contractAddress)) {
    contractElement.href = `${BASE_EXPLORER}/address/${catalog.contractAddress}`;
  }
}

function renderError(state: "OFFLINE" | "UNAVAILABLE"): void {
  rootElement.dataset.state = state.toLowerCase();
  rootElement.setAttribute("aria-busy", "false");
  listElement.setAttribute("aria-busy", "false");
  stateElement.textContent = state;
  sourceElement.textContent =
    state === "OFFLINE" ? "Reconnect to read the canonical Base contract" : "No validated Base snapshot available";
  summaryElement.textContent = "Acolyte NFT count unavailable";
  refreshElement.disabled = false;
  listElement.replaceChildren();
  emptyElement.hidden = true;
  errorElement.hidden = false;
}

function renderAcolyte(item: AcolyteCatalogItem): HTMLElement {
  const card = createElement("article", "acolyte-card");
  card.setAttribute("role", "listitem");

  const heading = createElement("div", "acolyte-card-heading");
  const title = document.createElement("h2");
  const expectedName = acolyteName(item.acolyte);
  const observedName = nftAcolyteName(item.traits);
  const nameMatches = observedName === expectedName;
  // The address-derived name is authoritative for this UI. The owner-controlled NFT trait is
  // displayed below as hostile metadata and contributes only a match/mismatch status badge.
  title.textContent = `Acolyte ${expectedName}`;
  const badges = createElement("div", "tentacle-badges");
  badges.append(
    textElement("span", item.status, `tentacle-badge ${item.status === "Active" ? "good" : "warn"}`),
    textElement("span", `token ${item.tokenId}`, "tentacle-badge"),
    textElement(
      "span",
      nameMatches ? "NFT name matches" : observedName ? "NFT name mismatch" : "NFT name missing",
      `tentacle-badge ${nameMatches ? "good" : "warn"}`,
    ),
  );
  heading.append(title, badges);

  const metrics = createElement("dl", "acolyte-metrics");
  appendMetric(metrics, "Acolyte", addressNode(item.acolyte));
  appendMetric(metrics, "Owner", addressNode(item.owner));
  // The catalog is an exact Branding-contract audit view. Canonical alias routing requires a
  // complete directory plus same-block registry verification and is performed by chat assignment;
  // never relabel this immutable stored value from an unauthoritative browser cache.
  appendMetric(metrics, "Stored on-chain controller", textNode(item.controllerAgentId));
  appendMetric(metrics, "Referrer", addressNode(item.referrer));
  appendMetric(metrics, "Declared price", textNode(formatUwu(item.declaredPrice)));
  appendMetric(metrics, "Paid through", timeNode(item.paidThrough));
  appendMetric(
    metrics,
    "Pending price",
    textNode(
      isZero(item.pendingDeclaredPrice)
        ? "none"
        : `${formatUwu(item.pendingDeclaredPrice)} after ${formatTimestamp(item.pendingPriceValidAfter)}`,
    ),
  );

  const metadata = createElement("div", "acolyte-metadata");
  const avatarLabel = textElement("strong", "Avatar URI");
  const avatar = safeHttpsUrl(item.avatarUri);
  const avatarValue = avatar
    ? linkElement(avatar.href, item.avatarUri, "acolyte-avatar-uri")
    : textElement("code", item.avatarUri || "not set", "acolyte-avatar-uri");
  metadata.append(avatarLabel, avatarValue);

  const traits = document.createElement("details");
  traits.className = "acolyte-traits";
  const traitsSummary = document.createElement("summary");
  traitsSummary.textContent = `${item.traits.length} owner trait${item.traits.length === 1 ? "" : "s"}`;
  const traitList = document.createElement("dl");
  for (const trait of item.traits) appendMetric(traitList, trait.traitType, textNode(trait.value));
  traits.append(traitsSummary, traitList);

  const provenance = createElement("p", "acolyte-provenance");
  provenance.append(document.createTextNode(`Minted at Base block ${item.mintBlockNumber}`));
  if (TRANSACTION_HASH.test(item.mintTransactionHash)) {
    provenance.append(
      document.createTextNode(" · "),
      linkElement(`${BASE_EXPLORER}/tx/${item.mintTransactionHash}`, "transaction"),
    );
  }

  card.append(heading, metrics, metadata, traits, provenance);
  return card;
}

function appendMetric(list: HTMLDListElement, label: string, value: Node): void {
  const group = document.createElement("div");
  const term = document.createElement("dt");
  const description = document.createElement("dd");
  term.textContent = label;
  description.append(value);
  group.append(term, description);
  list.append(group);
}

function addressNode(value: string): Node {
  return ADDRESS.test(value)
    ? linkElement(`${BASE_EXPLORER}/address/${value}`, shortAddress(value))
    : textNode(value || "unavailable");
}

function timeNode(value: string): Node {
  const formatted = formatTimestamp(value);
  if (formatted === "not set" || formatted === value) return textNode(formatted);
  const time = document.createElement("time");
  time.dateTime = formatted;
  time.textContent = formatted;
  return time;
}

function formatTimestamp(value: string): string {
  try {
    const seconds = BigInt(value);
    if (seconds === 0n || seconds > 8_640_000_000_000n) return seconds === 0n ? "not set" : value;
    return new Date(Number(seconds) * 1_000).toISOString();
  } catch {
    return value || "not set";
  }
}

function formatUwu(value: string): string {
  try {
    const raw = BigInt(value);
    const scale = 10n ** 18n;
    const whole = raw / scale;
    const fraction = (raw % scale).toString().padStart(18, "0").replace(/0+$/u, "");
    const visibleFraction = fraction.slice(0, 6);
    const suffix = fraction.length > visibleFraction.length ? "…" : "";
    return `${whole}${visibleFraction ? `.${visibleFraction}${suffix}` : ""} UWU`;
  } catch {
    return `${value || "unavailable"} UWU`;
  }
}

function safeHttpsUrl(value: string): URL | undefined {
  if (!value || value.length > 2_048) return undefined;
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || url.username || url.password) return undefined;
    return url;
  } catch {
    return undefined;
  }
}

function linkElement(href: string, label: string, className?: string): HTMLAnchorElement {
  const link = document.createElement("a");
  link.href = href;
  link.textContent = label;
  link.rel = "external noopener noreferrer nofollow";
  if (className) link.className = className;
  return link;
}

function textElement<K extends keyof HTMLElementTagNameMap>(
  tagName: K,
  value: string,
  className?: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tagName);
  element.textContent = value;
  if (className) element.className = className;
  return element;
}

function createElement<K extends keyof HTMLElementTagNameMap>(
  tagName: K,
  className: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tagName);
  element.className = className;
  return element;
}

function textNode(value: string): Text {
  return document.createTextNode(value);
}

function shortAddress(value: string): string {
  return ADDRESS.test(value) ? `${value.slice(0, 6)}…${value.slice(-4)}` : value || "unknown";
}

function isZero(value: string): boolean {
  try {
    return BigInt(value) === 0n;
  } catch {
    return false;
  }
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}
