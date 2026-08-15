import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
import { recruitmentUrl } from "../onboarding-links";
import { createXmtpWorkspace } from "./xmtp-workspace";
import {
  CHAT_CHANNELS,
  type ChatChannel,
  type ChatWorkspace,
  type WorkspaceSnapshot,
} from "./types";

const CHANNEL_LABELS: Record<ChatChannel, string> = {
  direct: "Direct",
  acolytes: "Acolytes",
  global: "Global",
};
const STATUS_COPY = {
  loading: "loading message history…",
  "awaiting-assignment": "waiting for the assigned Tentacle to provision this channel…",
  empty: "No messages here yet.",
  "policy-blocked": "This channel is paused until its trusted routing and retention policy can be verified.",
  error: "This channel is unavailable right now.",
} as const;

export interface ChatController {
  connect(focusComposer?: boolean): Promise<void>;
  resume(): Promise<void>;
  close(): Promise<void>;
}

interface ControllerDependencies {
  createWorkspace?: typeof createXmtpWorkspace;
}

interface ChatElements {
  root: HTMLElement;
  status: HTMLElement;
  name: HTMLElement;
  messages: HTMLDivElement;
  panel: HTMLElement;
  loadEarlier: HTMLButtonElement;
  newMessages: HTMLButtonElement;
  composer: HTMLFormElement;
  input: HTMLTextAreaElement;
  send: HTMLButtonElement;
  sendLabel: HTMLElement;
  retry: HTMLButtonElement;
  error: HTMLElement;
  retention: HTMLElement;
  composerLabel: HTMLLabelElement;
  tabs: Record<ChatChannel, HTMLButtonElement>;
  badges: Record<ChatChannel, HTMLElement>;
  activity: HTMLElement;
  rewardStatus: HTMLElement;
  copyReferral: HTMLButtonElement;
  referralStatus: HTMLElement;
  brandingDialog: HTMLDialogElement;
  brandingPrice: HTMLElement;
  brandingUpkeep: HTMLElement;
  brandingReferrer: HTMLElement;
  brandingAccept: HTMLButtonElement;
  brandingDecline: HTMLButtonElement;
}

type BrandingOffer = { messageId: string; price: bigint; upkeep: bigint };

export function initializeChatController(
  config: AppConfig,
  identity: StoredIdentity,
  dependencies: ControllerDependencies = {},
): ChatController {
  const elements = chatElements();
  const createWorkspace = dependencies.createWorkspace ?? createXmtpWorkspace;
  let workspace: ChatWorkspace | undefined;
  let unsubscribe: (() => void) | undefined;
  let connection: Promise<void> | undefined;
  let latest: WorkspaceSnapshot | undefined;
  let renderedChannel: ChatChannel | undefined;
  let sending = false;
  let loadingEarlier = false;
  let currentBrandingOffer: BrandingOffer | undefined;

  const updateComposerControls = (): void => {
    if (!latest) return;
    const channel = latest.channels[latest.activeChannel];
    const canSend = latest.connected && channel.retentionVerified &&
      (channel.status === "ready" || channel.status === "empty") &&
      Boolean(channel.writeConversationId);
    elements.input.disabled = !canSend || sending;
    elements.send.disabled = !canSend || sending || elements.input.value.trim().length === 0;
    elements.sendLabel.textContent = sending ? "sending…" : "whisper";
  };

  const render = (snapshot: WorkspaceSnapshot): void => {
    const previous = latest;
    latest = snapshot;
    const channelId = snapshot.activeChannel;
    const channel = snapshot.channels[channelId];
    const switched = renderedChannel !== channelId;
    const wasNearBottom = isNearBottom(elements.messages);
    const previousCount = previous?.channels[channelId].messages.length ?? 0;
    const received = !switched && channel.messages.length > previousCount;
    renderedChannel = channelId;

    elements.root.dataset.state = snapshot.connected ? "connected" : "retryable-error";
    elements.status.textContent = `${snapshot.assignmentNotice} · inbox ${shortId(snapshot.inboxId)}`;
    elements.name.textContent = channelId === "direct"
      ? snapshot.tentacleName
      : channelId === "acolytes" ? `${snapshot.tentacleName} · acolytes` : "Cthuwu · global";
    elements.copyReferral.disabled = !snapshot.assignedTentacleAddress;
    elements.panel.setAttribute("aria-labelledby", `tab-${channelId}`);
    elements.messages.setAttribute("aria-label", `${CHANNEL_LABELS[channelId]} channel messages`);
    elements.composerLabel.textContent = `Message the ${CHANNEL_LABELS[channelId]} channel`;

    for (const id of CHAT_CHANNELS) {
      const selected = id === channelId;
      const tab = elements.tabs[id];
      const unread = snapshot.channels[id].unread;
      tab.setAttribute("aria-selected", String(selected));
      tab.tabIndex = selected ? 0 : -1;
      elements.badges[id].textContent = unread > 99 ? "99+" : String(unread);
      elements.badges[id].hidden = unread === 0;
      tab.setAttribute(
        "aria-label",
        `${CHANNEL_LABELS[id]}${unread ? `, ${unread} unread` : ""}`,
      );
    }

    elements.messages.replaceChildren();
    elements.messages.classList.toggle("is-empty", channel.messages.length === 0 && !channel.typing);
    elements.messages.setAttribute("aria-busy", String(channel.status === "loading"));
    if (channel.hasMore) {
      elements.loadEarlier.hidden = false;
      elements.loadEarlier.disabled = channel.status === "loading";
    } else {
      elements.loadEarlier.hidden = true;
    }
    for (const message of channel.messages) {
      const bubble = document.createElement("article");
      bubble.className = `message ${message.mine ? "mine" : "theirs"}`;
      bubble.dataset.messageId = message.id;
      const sender = document.createElement("span");
      sender.className = "sender";
      sender.textContent = message.mine
        ? "You · acolyte"
        : channelId === "direct" ? snapshot.tentacleName : shortId(message.senderInboxId);
      const time = document.createElement("time");
      time.dateTime = nanosecondsToIso(message.sentAtNs);
      time.textContent = new Date(Number(message.sentAtNs / 1_000_000n)).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });
      const control = parseTentacleUi(message.id, message.text, message.mine, channelId);
      bubble.append(sender, document.createTextNode(control.text), time);
      elements.messages.append(bubble);
      if (control.reward) {
        elements.activity.hidden = false;
        elements.rewardStatus.textContent = control.reward.status === "confirmed"
          ? `${control.reward.amount.toLocaleString()} UWU reward confirmed on Base`
          : `${control.reward.amount.toLocaleString()} UWU reward queued · awaiting confirmed Base transfer`;
      }
      if (control.branding && !brandingDecision(control.branding.messageId)) {
        currentBrandingOffer = control.branding;
      }
    }
    if (channel.typing) {
      const indicator = document.createElement("div");
      indicator.className = "typing-indicator";
      indicator.setAttribute("role", "status");
      indicator.setAttribute("aria-live", "polite");
      indicator.setAttribute("aria-label", `${snapshot.tentacleName} is typing`);
      indicator.append(document.createElement("span"), document.createElement("span"), document.createElement("span"));
      elements.messages.append(indicator);
    }
    if (
      channel.messages.length > 0 &&
      (channel.status === "error" || channel.status === "policy-blocked")
    ) {
      const issue = document.createElement("div");
      issue.className = `channel-state channel-state-${channel.status}`;
      issue.setAttribute("role", "alert");
      issue.textContent = channel.error ?? STATUS_COPY[channel.status];
      elements.messages.prepend(issue);
    }
    if (channel.messages.length === 0 && !channel.typing) {
      const state = document.createElement("div");
      state.className = `channel-state channel-state-${channel.status}`;
      state.setAttribute("role", channel.status === "error" || channel.status === "policy-blocked" ? "alert" : "status");
      state.textContent = channel.error ?? STATUS_COPY[channel.status as keyof typeof STATUS_COPY] ?? "Channel ready.";
      elements.messages.append(state);
    }

    if (switched) {
      requestAnimationFrame(() => {
        elements.messages.scrollTop = workspace?.savedScrollTop(channelId) ?? 0;
      });
    } else if (received && wasNearBottom && !loadingEarlier) {
      requestAnimationFrame(() => scrollLatest(elements.messages));
    } else if (received && !loadingEarlier) {
      elements.newMessages.hidden = false;
    }

    updateComposerControls();
    if (currentBrandingOffer && !elements.brandingDialog.hasAttribute("open")) {
      elements.brandingPrice.textContent = `${currentBrandingOffer.price.toLocaleString()} base units`;
      elements.brandingUpkeep.textContent = `${currentBrandingOffer.upkeep.toLocaleString()} base units`;
      elements.brandingReferrer.textContent = config.referrer ?? "not supplied";
      elements.brandingDialog.hidden = false;
      elements.brandingDialog.setAttribute("open", "");
    }
    elements.retention.hidden = false;
    elements.retention.textContent = channel.retentionVerified
      ? "Messages disappear from supporting clients after 14 days."
      : "Composer locked until the 14-day disappearing-message policy is verified.";
    const assignmentRetryable = snapshot.assignmentState === "registry-unavailable" ||
      snapshot.assignmentState === "direct-verification-unavailable";
    elements.retry.hidden = snapshot.connected && !assignmentRetryable &&
      channel.status !== "error" && channel.status !== "policy-blocked";
  };

  const connect = async (focusComposer = false): Promise<void> => {
    if (connection) return connection;
    elements.root.dataset.state = "connecting";
    elements.status.textContent = "parting the veil through XMTP…";
    elements.input.disabled = true;
    connection = (async () => {
      try {
        await closeWorkspace();
        workspace = await createWorkspace(config, identity);
        unsubscribe = workspace.subscribe(render);
        if (focusComposer && !elements.input.disabled) elements.input.focus();
      } catch (error) {
        console.error(error);
        setError("the XMTP veil got tangled; your local identity is still here");
      } finally {
        connection = undefined;
      }
    })();
    return connection;
  };

  const activate = (channelId: ChatChannel, focus = false): void => {
    if (focus) elements.tabs[channelId].focus();
    if (!workspace) return;
    if (latest) {
      workspace.setViewport(
        latest.activeChannel,
        elements.messages.scrollTop,
        isNearBottom(elements.messages),
      );
    }
    workspace.setActiveChannel(channelId);
  };

  for (const id of CHAT_CHANNELS) {
    elements.tabs[id].addEventListener("click", () => activate(id));
    elements.tabs[id].addEventListener("keydown", (event) => {
      const current = CHAT_CHANNELS.indexOf(id);
      let next: number | undefined;
      if (event.key === "ArrowRight") next = (current + 1) % CHAT_CHANNELS.length;
      if (event.key === "ArrowLeft") next = (current - 1 + CHAT_CHANNELS.length) % CHAT_CHANNELS.length;
      if (event.key === "Home") next = 0;
      if (event.key === "End") next = CHAT_CHANNELS.length - 1;
      if (next === undefined) return;
      event.preventDefault();
      activate(CHAT_CHANNELS[next]!, true);
    });
  }
  elements.messages.addEventListener("scroll", () => {
    if (!workspace || !latest) return;
    const atBottom = isNearBottom(elements.messages);
    workspace.setViewport(latest.activeChannel, elements.messages.scrollTop, atBottom);
    if (atBottom) elements.newMessages.hidden = true;
  }, { passive: true });
  elements.newMessages.addEventListener("click", () => {
    scrollLatest(elements.messages);
    elements.newMessages.hidden = true;
  });
  elements.loadEarlier.addEventListener("click", () => {
    if (!workspace || !latest) return;
    const priorHeight = elements.messages.scrollHeight;
    const priorTop = elements.messages.scrollTop;
    loadingEarlier = true;
    elements.loadEarlier.disabled = true;
    void workspace.loadEarlier(latest.activeChannel).then(() => {
      requestAnimationFrame(() => {
        elements.messages.scrollTop = priorTop + (elements.messages.scrollHeight - priorHeight);
      });
    }).catch((error) => setComposerError(publicError(error))).finally(() => {
      loadingEarlier = false;
    });
  });
  elements.retry.addEventListener("click", () => {
    setComposerError();
    void (workspace ? workspace.revalidateAssignment("retry") : connect(true))
      .catch((error) => setComposerError(publicError(error)));
  });
  elements.input.addEventListener("input", () => {
    resize(elements.input);
    updateComposerControls();
  });
  elements.input.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      elements.composer.requestSubmit();
    }
  });
  elements.composer.addEventListener("submit", (event) => {
    event.preventDefault();
    if (!workspace || !latest || sending) return;
    const text = elements.input.value.trim();
    if (!text) return;
    sending = true;
    setComposerError();
    render(latest);
    void workspace.send(latest.activeChannel, text).then(() => {
      elements.input.value = "";
      resize(elements.input);
    }).catch((error) => setComposerError(publicError(error))).finally(() => {
      sending = false;
      if (latest) render(latest);
      elements.input.focus();
    });
  });

  const answerBrandingOffer = (accepted: boolean): void => {
    const offer = currentBrandingOffer;
    if (!offer || !workspace) return;
    localStorage.setItem(`cthuwu:branding-offer:v1:${offer.messageId}`, accepted ? "accepted" : "declined");
    elements.brandingDialog.removeAttribute("open");
    elements.brandingDialog.hidden = true;
    currentBrandingOffer = undefined;
    void workspace.send(
      "direct",
      accepted
        ? `I accept the Acolyte Branding offer shown in the Cthuwu app.${config.referrer ? ` Use referrer ${config.referrer} in the exact mint consent.` : ""}`
        : "I decline the Acolyte Branding offer for now.",
    ).catch((error) => setComposerError(publicError(error)));
  };
  elements.brandingAccept.addEventListener("click", () => answerBrandingOffer(true));
  elements.brandingDecline.addEventListener("click", () => answerBrandingOffer(false));
  elements.copyReferral.addEventListener("click", () => {
    const target = latest?.assignedTentacleAddress;
    if (!target) return;
    const url = recruitmentUrl(location.origin, target, identity.address);
    const copy = navigator.clipboard?.writeText(url);
    if (!copy) {
      elements.referralStatus.textContent = "could not copy the referral link";
      return;
    }
    void copy.then(() => {
      elements.referralStatus.textContent = "referral link copied";
    }).catch(() => {
      elements.referralStatus.textContent = "could not copy the referral link";
    });
  });

  const closeWorkspace = async (): Promise<void> => {
    unsubscribe?.();
    unsubscribe = undefined;
    const current = workspace;
    workspace = undefined;
    latest = undefined;
    if (current) await current.close().catch(() => undefined);
  };

  return {
    connect,
    resume: async () => {
      if (!workspace) return connect(false);
      await workspace.revalidateAssignment("resume");
    },
    close: async () => {
      await closeWorkspace();
    },
  };

  function setError(message: string): void {
    elements.root.dataset.state = "retryable-error";
    elements.status.textContent = message;
    elements.retry.hidden = false;
    elements.input.disabled = true;
    elements.send.disabled = true;
  }

  function setComposerError(message?: string): void {
    elements.error.hidden = !message;
    elements.error.textContent = message ?? "";
    elements.input.setAttribute("aria-invalid", String(Boolean(message)));
  }
}

function chatElements(): ChatElements {
  const tabs = Object.fromEntries(CHAT_CHANNELS.map((id) => [id, required<HTMLButtonElement>(`tab-${id}`)])) as Record<ChatChannel, HTMLButtonElement>;
  const badges = Object.fromEntries(CHAT_CHANNELS.map((id) => [id, required<HTMLElement>(`unread-${id}`)])) as Record<ChatChannel, HTMLElement>;
  const send = required<HTMLButtonElement>("send");
  return {
    root: required("chat"),
    status: required("status"),
    name: required("chat-name"),
    messages: required("messages"),
    panel: required("channel-panel"),
    loadEarlier: required("load-earlier"),
    newMessages: required("new-messages"),
    composer: required("composer"),
    input: required("message"),
    send,
    sendLabel: send.querySelector("span") ?? send,
    retry: required("connect"),
    error: required("composer-error"),
    retention: required("retention-notice"),
    composerLabel: document.querySelector<HTMLLabelElement>('label[for="message"]') ??
      (() => { throw new Error("Missing composer label"); })(),
    tabs,
    badges,
    activity: required("activity-card"),
    rewardStatus: required("reward-status"),
    copyReferral: required("copy-referral"),
    referralStatus: required("referral-status"),
    brandingDialog: required("branding-offer"),
    brandingPrice: required("branding-price"),
    brandingUpkeep: required("branding-upkeep"),
    brandingReferrer: required("branding-referrer"),
    brandingAccept: required("branding-accept"),
    brandingDecline: required("branding-decline"),
  };
}

function parseTentacleUi(
  messageId: string,
  text: string,
  mine: boolean,
  channel: ChatChannel,
): {
  text: string;
  reward?: { status: "pending" | "confirmed"; amount: bigint };
  branding?: BrandingOffer;
} {
  if (mine || channel !== "direct") return { text };
  let visible = text;
  let reward: { status: "pending" | "confirmed"; amount: bigint } | undefined;
  let branding: BrandingOffer | undefined;
  visible = visible.replace(
    /\n?\[\[cthuwu:reward:v1;status=(pending|confirmed);amount=([1-9][0-9]{0,19})\]\]/gu,
    (_match, status: "pending" | "confirmed", amount: string) => {
      reward = { status, amount: BigInt(amount) };
      return "";
    },
  );
  visible = visible.replace(
    /\n?\[\[cthuwu:branding-offer:v1;treasury=(0x[0-9a-f]+);price=(0x[0-9a-f]+);upkeep=(0x[0-9a-f]+)\]\]/gu,
    (_match, _treasury: string, price: string, upkeep: string) => {
      branding = { messageId, price: BigInt(price), upkeep: BigInt(upkeep) };
      return "";
    },
  );
  return { text: visible.trimEnd(), ...(reward ? { reward } : {}), ...(branding ? { branding } : {}) };
}

function brandingDecision(messageId: string): boolean {
  return localStorage.getItem(`cthuwu:branding-offer:v1:${messageId}`) !== null;
}

function required<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing required element #${id}`);
  return value as T;
}

function isNearBottom(element: HTMLElement): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight < 72;
}

function scrollLatest(element: HTMLElement): void {
  element.scrollTo({ top: element.scrollHeight, behavior: "smooth" });
}

function resize(input: HTMLTextAreaElement): void {
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, 144)}px`;
}

function shortId(value: string): string {
  return value.length <= 16 ? value : `${value.slice(0, 8)}…${value.slice(-6)}`;
}

function nanosecondsToIso(value: bigint): string {
  try {
    return new Date(Number(value / 1_000_000n)).toISOString();
  } catch {
    return "";
  }
}

function publicError(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "the void did not answer";
}
