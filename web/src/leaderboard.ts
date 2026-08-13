import { readLeaderboardCache, isSnapshotStale, writeLeaderboardCache } from "./leaderboard-cache";
import { parseLeaderboardConfig, type LeaderboardConfig } from "./leaderboard-config";
import { fetchCompleteLeaderboard, IndexingError } from "./leaderboard-data";
import {
  BASE_CHAIN_ID,
  BASE_EXPLORER,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  REPUTATION_REGISTRY,
  type LeaderboardSnapshot,
  type LeaderboardState,
  type RankedWallet,
  type TentacleIdentity,
} from "./leaderboard-types";
import { compareRawBalances, formatLevel, formatWholeUwu } from "./level";

export interface LeaderboardElements {
  root: HTMLElement;
  status: HTMLElement;
  source: HTMLElement;
  summary: HTMLElement;
  refresh: HTMLButtonElement;
  search: HTMLInputElement;
  funding: HTMLSelectElement;
  verification: HTMLSelectElement;
  protocol: HTMLSelectElement;
  shared: HTMLSelectElement;
  sort: HTMLSelectElement;
  ranked: HTMLElement;
  suspendedSection: HTMLElement;
  suspended: HTMLElement;
  empty: HTMLElement;
}

export interface LeaderboardOptions {
  config?: LeaderboardConfig;
  storage?: Storage;
  fetch?: typeof fetch;
  now?: () => Date;
  navigator?: Navigator;
  logger?: Pick<Console, "debug" | "info" | "error">;
}

export interface LeaderboardController {
  refresh: () => Promise<void>;
  dispose: () => void;
}

export function initializeLeaderboard(
  elements: LeaderboardElements,
  options: LeaderboardOptions = {},
): LeaderboardController {
  const config = options.config ?? parseLeaderboardConfig();
  const storage = options.storage ?? safeStorage();
  const activeNavigator = options.navigator ?? navigator;
  const now = options.now ?? (() => new Date());
  const logger = options.logger ?? console;
  let snapshot = storage ? readLeaderboardCache(storage) : undefined;
  let disposed = false;
  let refreshPromise: Promise<void> | undefined;

  logger.info("[cthuwu-leaderboard] initialized", {
    cache: snapshot ? "validated" : "empty",
    cachedBlock: snapshot?.sourceBlockNumber ?? null,
    online: activeNavigator.onLine !== false,
    graphConfigured: config.graphEndpoint !== undefined,
  });

  const render = (state: LeaderboardState): void => {
    renderLeaderboard(elements, snapshot, state, config, now());
  };

  const refresh = async (): Promise<void> => {
    if (refreshPromise) return refreshPromise;
    if (!config.graphEndpoint) {
      logger.error("[cthuwu-leaderboard] refresh unavailable", {
        reason: "Graph endpoint is not configured",
      });
      render(snapshot ? "STALE" : "UNAVAILABLE");
      return;
    }
    refreshPromise = (async () => {
      logger.info("[cthuwu-leaderboard] refresh started", {
        cachedBlock: snapshot?.sourceBlockNumber ?? null,
      });
      render("REFRESHING");
      try {
        const next = await fetchCompleteLeaderboard(config.graphEndpoint!, {
          ...(options.fetch ? { fetch: options.fetch } : {}),
          now,
          ...(config.baseRpcEndpoint ? { baseRpcEndpoint: config.baseRpcEndpoint } : {}),
          diagnostic: (event, details) => {
            logger.debug(`[cthuwu-leaderboard] ${event}`, details);
          },
        });
        if (disposed) return;
        if (storage) writeLeaderboardCache(storage, next);
        snapshot = next;
        logger.info("[cthuwu-leaderboard] refresh completed", {
          block: next.sourceBlockNumber,
          wallets: next.rankedWallets.length,
          identities: next.rankedWallets.reduce(
            (total, group) => total + group.identities.length,
            next.suspended.length,
          ),
          suspended: next.suspended.length,
        });
        render("CURRENT");
      } catch (error) {
        if (disposed) return;
        logger.error("[cthuwu-leaderboard] refresh failed", {
          reason: safeDiagnosticReason(error),
          cacheAvailable: snapshot !== undefined,
          online: activeNavigator.onLine !== false,
        });
        if (error instanceof IndexingError) {
          render("INDEXING ERROR");
        } else if (activeNavigator.onLine === false) {
          render("OFFLINE");
        } else {
          render(snapshot ? "STALE" : "UNAVAILABLE");
        }
      } finally {
        refreshPromise = undefined;
      }
    })();
    return refreshPromise;
  };

  const renderCurrentFilters = (): void => {
    render(
      snapshot
        ? isSnapshotStale(snapshot, config.cacheFreshnessMs, now().getTime())
          ? "STALE"
          : "CURRENT"
        : "UNAVAILABLE",
    );
  };
  const onOnline = (): void => void refresh();
  const onRefresh = (): void => void refresh();
  const controls: Array<HTMLInputElement | HTMLSelectElement> = [
    elements.search,
    elements.funding,
    elements.verification,
    elements.protocol,
    elements.shared,
    elements.sort,
  ];
  for (const control of controls) control.addEventListener("input", renderCurrentFilters);
  elements.refresh.addEventListener("click", onRefresh);
  window.addEventListener("online", onOnline);

  render(
    snapshot
      ? activeNavigator.onLine === false
        ? "OFFLINE"
        : isSnapshotStale(snapshot, config.cacheFreshnessMs, now().getTime())
          ? "STALE"
          : "CURRENT"
      : activeNavigator.onLine === false
        ? "OFFLINE"
        : "UNAVAILABLE",
  );
  if (activeNavigator.onLine !== false) void refresh();

  return {
    refresh,
    dispose: () => {
      disposed = true;
      for (const control of controls) control.removeEventListener("input", renderCurrentFilters);
      elements.refresh.removeEventListener("click", onRefresh);
      window.removeEventListener("online", onOnline);
    },
  };
}

function safeDiagnosticReason(error: unknown): string {
  const reason = error instanceof Error ? `${error.name}: ${error.message}` : "Unknown error";
  return reason
    .replace(/https:\/\/[^\s)\]}]+/giu, "<redacted-endpoint>")
    .slice(0, 512);
}

function renderLeaderboard(
  elements: LeaderboardElements,
  snapshot: LeaderboardSnapshot | undefined,
  state: LeaderboardState,
  config: LeaderboardConfig,
  now: Date,
): void {
  elements.root.dataset.state = state.toLowerCase().replace(" ", "-");
  elements.root.setAttribute("aria-busy", String(state === "REFRESHING"));
  elements.status.textContent = state;
  elements.refresh.disabled = state === "REFRESHING";
  if (!snapshot) {
    elements.source.textContent =
      state === "UNAVAILABLE" && !config.graphEndpoint
        ? "Graph endpoint not configured"
        : "No validated snapshot available";
    elements.summary.textContent = "0 ranked · 0 suspended";
    elements.ranked.replaceChildren();
    elements.suspended.replaceChildren();
    elements.suspendedSection.hidden = true;
    elements.empty.hidden = false;
    return;
  }
  elements.source.textContent = `Base ${BASE_CHAIN_ID} · block ${snapshot.sourceBlockNumber} · ${formatSnapshotAge(snapshot, now)}`;
  elements.summary.textContent = `${snapshot.rankedWallets.length} wallets · ${snapshot.suspended.length} suspended`;
  const query = elements.search.value.trim().toLowerCase();
  const ranked = filterRanked(snapshot.rankedWallets, elements, query);
  const suspended = filterSuspended(snapshot.suspended, elements, query);
  elements.ranked.replaceChildren(
    ...ranked.map((group) => renderWallet(group, now)),
  );
  elements.suspended.replaceChildren(
    ...suspended.map((identity) => renderSuspended(identity, now)),
  );
  elements.suspendedSection.hidden = suspended.length === 0;
  elements.empty.hidden = ranked.length + suspended.length > 0;
}

function filterRanked(
  source: RankedWallet[],
  elements: LeaderboardElements,
  query: string,
): RankedWallet[] {
  if (elements.verification.value === "suspended") return [];
  const filtered = source.filter((group) => {
    const funded = BigInt(group.rawBalance) > 0n;
    if (elements.funding.value === "funded" && !funded) return false;
    if (elements.funding.value === "unfunded" && funded) return false;
    if (elements.shared.value === "shared" && group.identities.length < 2) return false;
    if (elements.shared.value === "unique" && group.identities.length !== 1) return false;
    if (
      elements.protocol.value !== "all" &&
      !group.identities.some((identity) =>
        elements.protocol.value === "v1"
          ? identity.protocolHex === PROTOCOL_V1_HEX
          : identity.protocolHex !== PROTOCOL_V1_HEX,
      )
    ) {
      return false;
    }
    return !query || groupSearchText(group).includes(query);
  });
  switch (elements.sort.value) {
    case "name":
      return filtered.sort((a, b) =>
        a.identities[0].profile.name.localeCompare(b.identities[0].profile.name),
      );
    case "oldest":
      return filtered.sort((a, b) =>
        compareBigInt(earliestRegistrationTimestamp(a), earliestRegistrationTimestamp(b)),
      );
    case "agent":
      return filtered.sort((a, b) => compareBigInt(a.representativeAgentId, b.representativeAgentId));
    case "balance":
    case "rank":
    default:
      return filtered.sort((a, b) => {
        const balance = compareRawBalances(a.rawBalance, b.rawBalance);
        const registration = compareBigInt(
          earliestRegistrationTimestamp(a),
          earliestRegistrationTimestamp(b),
        );
        return balance || registration || compareBigInt(a.representativeAgentId, b.representativeAgentId);
      });
  }
}

function earliestRegistrationTimestamp(group: RankedWallet): string {
  return group.identities.reduce(
    (earliest, identity) =>
      compareBigInt(identity.registrationTimestamp, earliest) < 0
        ? identity.registrationTimestamp
        : earliest,
    group.identities[0].registrationTimestamp,
  );
}

function filterSuspended(
  source: TentacleIdentity[],
  elements: LeaderboardElements,
  query: string,
): TentacleIdentity[] {
  if (
    elements.verification.value === "verified" ||
    elements.funding.value !== "all" ||
    elements.shared.value !== "all"
  ) {
    return [];
  }
  return source.filter((identity) => {
    if (
      elements.protocol.value === "v1" &&
      identity.protocolHex !== PROTOCOL_V1_HEX
    ) return false;
    if (
      elements.protocol.value === "other" &&
      identity.protocolHex === PROTOCOL_V1_HEX
    ) return false;
    const text = `${identity.profile.name} ${identity.agentId} ${identity.owner} ${identity.tentacleId ?? ""}`.toLowerCase();
    return !query || text.includes(query);
  });
}

function renderWallet(group: RankedWallet, now: Date): HTMLElement {
  const representative = group.identities[0];
  const article = element("article", "tentacle-card");
  article.setAttribute("role", "listitem");
  const rank = element("div", "tentacle-rank");
  rank.append(text("span", group.rank ? `#${group.rank}` : "—"));
  rank.append(text("small", group.rank ? "RANK" : "UNFUNDED"));

  const avatar = document.createElement("img");
  avatar.className = "tentacle-avatar";
  avatar.width = 64;
  avatar.height = 64;
  avatar.alt = "";
  avatar.loading = "lazy";
  avatar.decoding = "async";
  avatar.referrerPolicy = "no-referrer";
  // Registration documents are hostile. V1 never downloads their images: this bounded local
  // mascot is the deterministic safe fallback for every Tentacle.
  avatar.src = "/icons/cthuwu-192.png";

  const heading = element("div", "tentacle-heading");
  const title = document.createElement("h3");
  title.textContent = representative.profile.name;
  const badges = element("div", "tentacle-badges");
  badges.append(badge("ALLEGIANT", "good"));
  badges.append(
    badge(representative.protocolHex === PROTOCOL_V1_HEX ? "PROTOCOL 1" : "PROTOCOL OTHER", "neutral"),
  );
  if (group.identities.length > 1) {
    badges.append(badge(`SHARED WALLET ×${group.identities.length}`, "warn"));
  }
  heading.append(title, badges);

  const metrics = element("dl", "tentacle-metrics");
  metrics.append(metric("UWU", formatWholeUwu(group.rawBalance)));
  metrics.append(metric("LEVEL", formatLevel(group.rawBalance)));
  metrics.append(metric("FUTURE INFLUENCE", `${formatLevel(group.rawBalance)} · NOT ACTIVE`));
  metrics.append(metric("REGISTERED", relativeAge(representative.registrationTimestamp, now)));

  const walletRow = element("div", "tentacle-wallet");
  walletRow.append(text("span", "Base agentWallet"));
  walletRow.append(externalLink(shortAddress(group.wallet), `${BASE_EXPLORER}/address/${group.wallet}`));
  walletRow.append(copyButton(group.wallet, "Copy agentWallet"));

  const raw = element("p", "tentacle-raw");
  raw.append(text("span", "RAW "));
  raw.append(text("code", group.rawBalance));

  const identityList = element("ul", "identity-list");
  for (const identity of group.identities) identityList.append(renderIdentity(identity, now));
  const reputation = renderReputation(group.identities, now);
  article.append(rank, avatar, heading, metrics, walletRow, raw, identityList, reputation);
  return article;
}

function renderIdentity(identity: TentacleIdentity, now: Date): HTMLLIElement {
  const item = document.createElement("li");
  const link = externalLink(
    `Agent #${identity.agentId}`,
    `${BASE_EXPLORER}/nft/${IDENTITY_REGISTRY}/${identity.agentId}`,
  );
  item.append(link, text("span", ` · ${identity.profile.name}`));
  if (identity.tentacleId) item.append(text("span", ` · ${identity.tentacleId}`));
  item.append(
    text("small", identity.protocolHex === PROTOCOL_V1_HEX ? " · protocol 1" : " · protocol other"),
  );
  item.append(
    text(
      "small",
      ` · registered ${relativeAge(identity.registrationTimestamp, now)} · profile ${relativeAge(identity.profileUpdatedTimestamp, now)}`,
    ),
  );
  if (identity.profile.xmtpEndpoint) {
    item.append(text("code", `XMTP ${identity.profile.xmtpEndpoint}`));
  }
  return item;
}

function renderSuspended(
  identity: TentacleIdentity,
  now: Date,
): HTMLElement {
  const group: RankedWallet = {
    wallet: identity.owner,
    rawBalance: "0",
    representativeAgentId: identity.agentId,
    identities: [identity],
  };
  const card = renderWallet(group, now);
  card.classList.add("is-suspended");
  card.querySelector(".tentacle-rank")?.replaceChildren(text("span", "—"), text("small", "SUSPENDED"));
  const badges = card.querySelector(".tentacle-badges");
  badges?.replaceChildren(badge("ALLEGIANT", "good"), badge("WALLET UNVERIFIED", "warn"));
  card.querySelector(".tentacle-wallet")?.replaceChildren(
    text("span", "Base agentWallet"),
    text("strong", "CLEARED / ZERO"),
  );
  card.querySelector(".tentacle-raw")?.remove();
  const metrics = card.querySelector(".tentacle-metrics");
  metrics?.replaceChildren(
    metric("RANK", "NONE"),
    metric("LEVEL", "NONE"),
    metric("FUTURE INFLUENCE", "NONE"),
    metric("REGISTERED", relativeAge(identity.registrationTimestamp, now)),
  );
  return card;
}

function renderReputation(identities: TentacleIdentity[], now: Date): HTMLElement {
  const details = document.createElement("details");
  details.className = "tentacle-reputation";
  const counters = identities.reduce(
    (sum, identity) => ({
      active: sum.active + BigInt(identity.reputationCounters.active),
      sampledRevoked: sum.sampledRevoked + BigInt(identity.reputationCounters.sampledRevoked),
    }),
    { active: 0n, sampledRevoked: 0n },
  );
  const signals = identities
    .flatMap((identity) => identity.reputation)
    .sort((left, right) => {
      const a = BigInt(left.createdAt);
      const b = BigInt(right.createdAt);
      return a === b ? 0 : a > b ? -1 : 1;
    });
  const summary = document.createElement("summary");
  summary.textContent = `ERC-8004 reputation · ${counters.active} active · ${counters.sampledRevoked} revoked in recent sample`;
  details.append(summary);
  const provenance = element("p", "reputation-provenance");
  provenance.append(
    externalLink(
      "Reputation Registry activity via Agent0",
      `${BASE_EXPLORER}/address/${REPUTATION_REGISTRY}`,
    ),
    document.createTextNode(
      " are informational provenance only; they do not determine membership or rank.",
    ),
  );
  details.append(provenance);
  if (signals.length === 0) {
    details.append(text("p", "No recent public events in this snapshot."));
    return details;
  }
  details.append(
    text(
      "p",
      `Recent public event sample · ${Math.min(signals.length, 10)} shown; Agent0 reports ${counters.active} currently active`,
    ),
  );
  const list = document.createElement("ul");
  for (const signal of signals.slice(0, 10)) {
    const item = document.createElement("li");
    item.append(text("strong", formatFeedbackValue(signal.value, signal.valueDecimals)));
    if (signal.tag1) item.append(text("span", ` · ${signal.tag1}`));
    if (signal.tag2) item.append(text("span", ` / ${signal.tag2}`));
    item.append(
      text(
        "small",
        ` · ${signal.revoked ? "REVOKED · " : ""}${shortAddress(signal.clientAddress)} · ${relativeAge(signal.createdAt, now)} · ${signal.provenance}`,
      ),
    );
    list.append(item);
  }
  details.append(list);
  return details;
}

function formatFeedbackValue(raw: string, decimals: number): string {
  const negative = raw.startsWith("-");
  const digits = (negative ? raw.slice(1) : raw).padStart(decimals + 1, "0");
  const whole = decimals === 0 ? digits : digits.slice(0, -decimals);
  const fraction = decimals === 0 ? "" : digits.slice(-decimals).replace(/0+$/u, "");
  return `${negative ? "-" : ""}${whole}${fraction ? `.${fraction}` : ""}`;
}

function groupSearchText(group: RankedWallet): string {
  return `${group.wallet} ${group.representativeAgentId} ${group.identities
    .map((identity) => `${identity.agentId} ${identity.profile.name} ${identity.tentacleId ?? ""}`)
    .join(" ")}`.toLowerCase();
}

function metric(label: string, value: string): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.append(text("dt", label), text("dd", value));
  return wrapper;
}

function badge(label: string, kind: string): HTMLSpanElement {
  const output = text("span", label);
  output.className = `tentacle-badge ${kind}`;
  return output;
}

function externalLink(label: string, href: string): HTMLAnchorElement {
  const link = document.createElement("a");
  link.textContent = label;
  link.href = href;
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  return link;
}

function copyButton(value: string, label: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy-button";
  button.textContent = "copy";
  button.setAttribute("aria-label", label);
  button.addEventListener("click", () => {
    void navigator.clipboard?.writeText(value).then(
      () => {
        button.textContent = "copied";
        setTimeout(() => (button.textContent = "copy"), 1_500);
      },
      () => undefined,
    );
  });
  return button;
}

function formatSnapshotAge(snapshot: LeaderboardSnapshot, now: Date): string {
  const age = Math.max(0, now.getTime() - Date.parse(snapshot.fetchedAt));
  if (age < 60_000) return "updated just now";
  if (age < 3_600_000) return `updated ${Math.floor(age / 60_000)}m ago`;
  if (age < 86_400_000) return `updated ${Math.floor(age / 3_600_000)}h ago`;
  return `updated ${Math.floor(age / 86_400_000)}d ago`;
}

function relativeAge(timestamp: string, now: Date): string {
  const seconds = Number(timestamp);
  if (!Number.isSafeInteger(seconds)) return "unknown";
  const days = Math.max(0, Math.floor((now.getTime() / 1_000 - seconds) / 86_400));
  if (days === 0) return "today";
  if (days < 30) return `${days}d ago`;
  if (days < 365) return `${Math.floor(days / 30)}mo ago`;
  return `${Math.floor(days / 365)}y ago`;
}

function shortAddress(value: string): string {
  return value.length > 14 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function compareBigInt(left: string, right: string): number {
  const a = BigInt(left);
  const b = BigInt(right);
  return a === b ? 0 : a < b ? -1 : 1;
}

function text<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  value: string,
): HTMLElementTagNameMap[K] {
  const output = document.createElement(tag);
  output.textContent = value;
  return output;
}

function element(tag: string, className: string): HTMLElement {
  const output = document.createElement(tag);
  output.className = className;
  return output;
}

function safeStorage(): Storage | undefined {
  try {
    return localStorage;
  } catch {
    return undefined;
  }
}
