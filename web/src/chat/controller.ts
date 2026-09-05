import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
import { acolyteName } from "../acolyte-name";
import { recruitmentUrl } from "../onboarding-links";
import {
  consentMatchesOffer,
  encodeBrandingDecline,
  encodeBrandingRequest,
  parseBrandingMessage,
  reviewBrandingOffer,
  signBrandingOffer,
  verifyBrandingReceipt,
  type BrandingConsent,
  type BrandingOffer,
  type BrandingReceipt,
  type BrandingReview,
} from "../branding-consent";
import { createXmtpWorkspace } from "./xmtp-workspace";
import {
  CHAT_CHANNELS,
  type ChannelSnapshot,
  type ChatChannel,
  type ChatWorkspace,
  type WorkspaceMessage,
  type WorkspaceSnapshot,
} from "./types";

const CHANNEL_LABELS: Record<ChatChannel, string> = {
  direct: "Direct",
  acolytes: "Acolytes",
  global: "Global",
};
const OPERATOR_GROWTH_REGISTRATION = "[[cthuwu:growth-operator-register:v1]]";
const OPERATOR_GROWTH_ACK = "[[cthuwu:growth-operator-ack:v1]]";
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
  brandingOffers?: boolean;
  surface?: "public" | "operator";
  reviewBrandingOffer?: typeof reviewBrandingOffer;
  signBrandingOffer?: typeof signBrandingOffer;
  verifyBrandingReceipt?: typeof verifyBrandingReceipt;
  nowSeconds?: () => bigint;
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
  brandingReview: HTMLButtonElement;
  brandingStart: HTMLButtonElement;
  copyReferral: HTMLButtonElement;
  referralStatus: HTMLElement;
  brandingDialog: HTMLDialogElement;
  brandingName: HTMLElement;
  brandingPrice: HTMLElement;
  brandingUpkeep: HTMLElement;
  brandingReferrer: HTMLElement;
  brandingContract: HTMLElement;
  brandingMinter: HTMLElement;
  brandingAgent: HTMLElement;
  brandingBasis: HTMLElement;
  brandingNonce: HTMLElement;
  brandingDeadline: HTMLElement;
  brandingStatus: HTMLElement;
  brandingAccept: HTMLButtonElement;
  brandingDecline: HTMLButtonElement;
}

type BrandingStage = "offer" | "checking" | "review" | "signing" | "pending" | "complete" | "error";

export function initializeChatController(
  config: AppConfig,
  identity: StoredIdentity,
  dependencies: ControllerDependencies = {},
): ChatController {
  const elements = chatElements();
  const verificationNotice = document.createElement("aside");
  verificationNotice.className = "verification-notice";
  verificationNotice.setAttribute("aria-label", "Tentacle verification");
  verificationNotice.hidden = true;
  const verificationText = document.createElement("p");
  verificationText.setAttribute("role", "status");
  const dismissVerification = document.createElement("button");
  dismissVerification.type = "button";
  dismissVerification.textContent = "Dismiss";
  let dismissedWarning: string | undefined;
  dismissVerification.addEventListener("click", () => {
    dismissedWarning = latest?.verificationWarning;
    verificationNotice.hidden = true;
  });
  verificationNotice.append(verificationText, dismissVerification);
  elements.root.after(verificationNotice);
  const createWorkspace = dependencies.createWorkspace ?? createXmtpWorkspace;
  let workspace: ChatWorkspace | undefined;
  let unsubscribe: (() => void) | undefined;
  let connection: Promise<void> | undefined;
  let latest: WorkspaceSnapshot | undefined;
  let canonicalGrowthReferrer: string | undefined;
  let renderedChannel: ChatChannel | undefined;
  let sending = false;
  let loadingEarlier = false;
  let currentBrandingOffer: BrandingOffer | undefined;
  let currentBrandingConsent: BrandingConsent | undefined;
  let currentBrandingReceipt: BrandingReceipt | undefined;
  let currentBrandingReview: BrandingReview | undefined;
  let brandingStage: BrandingStage = "offer";
  let brandingError: string | undefined;
  let brandingBusy = false;
  const requestedReplacementOffers = new Set<string>();
  const verifiedReceiptBindings = new Set<string>();
  const verifyingReceiptBindings = new Set<string>();
  const failedReceiptBindings = new Map<string, string>();
  const localAcolyteName = acolyteName(identity.address);
  const operatorSurface = dependencies.surface === "operator";
  const operatorTarget = `Target Tentacle · ${shortId(config.botAddress)}`;
  const activeChannel = (snapshot: WorkspaceSnapshot): ChatChannel =>
    operatorSurface ? "direct" : snapshot.activeChannel;
  const nowSeconds = dependencies.nowSeconds ?? (() => BigInt(Math.floor(Date.now() / 1000)));
  if (!operatorSurface && config.referrer) {
    elements.referralStatus.textContent =
      `Invited by ${shortId(config.referrer)}. Start by chatting about what you want help with. Branding is optional and requires a separate review and signature; referral rewards are confirmed only after eligible onboarding.`;
  }

  const updateComposerControls = (): void => {
    if (!latest) return;
    const channel = latest.channels[activeChannel(latest)];
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
    const channelId = activeChannel(snapshot);
    const channel = snapshot.channels[channelId];
    const switched = renderedChannel !== channelId;
    const wasNearBottom = isNearBottom(elements.messages);
    const previousCount = previous?.channels[channelId].messages.length ?? 0;
    const received = !switched && channel.messages.length > previousCount;
    renderedChannel = channelId;
    const referralAcknowledgement = latestReferralAcknowledgement(
      snapshot.channels.direct.messages,
    );
    if (referralAcknowledgement) {
      canonicalGrowthReferrer = referralAcknowledgement.referrer === "none"
        ? identity.address.toLowerCase()
        : referralAcknowledgement.referrer;
    }

    verificationText.textContent = snapshot.verificationWarning ?? "";
    verificationNotice.hidden = !snapshot.verificationWarning || snapshot.verificationWarning === dismissedWarning;
    elements.root.dataset.state = snapshot.connected ? "connected" : "retryable-error";
    const verifiedOperatorDirect = snapshot.connected && channelId === "direct" &&
      channel.retentionVerified && Boolean(channel.writeConversationId) &&
      (channel.status === "ready" || channel.status === "empty");
    elements.status.textContent = operatorSurface
      ? `${verifiedOperatorDirect ? "Verified direct XMTP route; 14-day retention active" : channel.error ?? (snapshot.connected ? STATUS_COPY[channel.status as keyof typeof STATUS_COPY] ?? "Direct route unavailable" : "Direct XMTP route disconnected")} · operator inbox ${shortId(snapshot.inboxId)}`
      : `${snapshot.assignmentNotice} · inbox ${shortId(snapshot.inboxId)}`;
    elements.name.textContent = operatorSurface ? operatorTarget : channelId === "direct"
      ? snapshot.tentacleName
      : channelId === "acolytes" ? `${snapshot.tentacleName} · acolytes` : "Cthuwu · global";
    elements.copyReferral.disabled = !snapshot.assignedTentacleAddress;
    const brandingCanBeCompleted = snapshot.assignmentState === "liveness-required" ||
      snapshot.assignmentState === "anchor-verified" ||
      snapshot.assignmentState === "rotation-verified";
    elements.brandingStart.hidden =
      operatorSurface || !brandingCanBeCompleted || !snapshot.connected;
    if (!elements.brandingStart.hidden && elements.brandingReview.hidden) {
      elements.activity.hidden = false;
      if (!elements.rewardStatus.textContent) {
        elements.rewardStatus.textContent =
          "Unbranded acolyte · review what Branding does and its exact UWU cost before consenting.";
      }
    }
    elements.panel.setAttribute("aria-labelledby", `tab-${channelId}`);
    elements.messages.setAttribute("aria-label", operatorSurface
      ? "Direct operator messages"
      : `${CHANNEL_LABELS[channelId]} channel messages`);
    elements.composerLabel.textContent = operatorSurface
      ? "Message the direct operator channel"
      : `Message the ${CHANNEL_LABELS[channelId]} channel`;

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
    let latestOffer: BrandingOffer | undefined;
    let latestConsent: BrandingConsent | undefined;
    let latestReceipt: BrandingReceipt | undefined;
    let latestDeclinedOfferId: string | undefined;
    let latestRequest: { referrer: string; name: string } | undefined;
    for (const message of channel.messages) {
      const bubble = document.createElement("article");
      bubble.className = `message ${message.mine ? "mine" : "theirs"}`;
      bubble.dataset.messageId = message.id;
      const sender = document.createElement("span");
      sender.className = "sender";
      sender.textContent = message.mine
        ? operatorSurface ? `You · ${localAcolyteName} · operator` : `You · ${localAcolyteName}`
        : operatorSurface ? operatorTarget
          : channelId === "direct" ? snapshot.tentacleName : shortId(message.senderInboxId);
      const time = document.createElement("time");
      time.dateTime = nanosecondsToIso(message.sentAtNs);
      time.textContent = new Date(Number(message.sentAtNs / 1_000_000n)).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });
      // Operator/tool output is literal text. Public UI markers have no control meaning here and
      // must never be removed from an auditable privileged transcript.
      const operatorGrowthControl = operatorSurface &&
        (message.text === OPERATOR_GROWTH_REGISTRATION || message.text === OPERATOR_GROWTH_ACK);
      const branding = operatorSurface || channelId !== "direct"
        ? { text: operatorGrowthControl ? "" : message.text }
        : parseBrandingMessage(message.text, message.mine ? "mine" : "theirs");
      const control = operatorSurface
        ? { text: branding.text }
        : parseTentacleUi(branding.text, message.mine, channelId);
      bubble.append(sender, document.createTextNode(control.text), time);
      // Exact Branding controls are transport state, not chat prose. Keep an otherwise empty
      // control message out of the transcript while preserving malformed, duplicated, misplaced,
      // or wrong-direction markers literally.
      if (control.text.length > 0) elements.messages.append(bubble);
      if (control.reward) {
        elements.activity.hidden = false;
        const amount = control.reward.baseUnits
          ? formatUwuDisplay(control.reward.amount)
          : `${control.reward.amount.toLocaleString()} UWU`;
        const label = control.reward.baseUnits ? "referral reward" : "reward";
        elements.rewardStatus.textContent = control.reward.status === "confirmed"
          ? `${amount} ${label} confirmed on Base`
          : `${amount} ${label} queued · awaiting confirmed Base transfer`;
      }
      if (dependencies.brandingOffers !== false && branding.control) {
        switch (branding.control.type) {
          case "offer":
            if (latestOffer?.marker !== branding.control.marker) {
              latestConsent = undefined;
              latestReceipt = undefined;
              latestDeclinedOfferId = undefined;
            }
            latestOffer = branding.control;
            break;
          case "consent":
            if (latestOffer && consentMatchesOffer(branding.control, latestOffer)) {
              latestConsent = branding.control;
            }
            break;
          case "receipt":
            if (latestOffer && branding.control.offerId === latestOffer.offerId) {
              latestReceipt = branding.control;
            }
            break;
          case "decline":
            latestDeclinedOfferId = branding.control.offerId;
            break;
          case "request":
            latestRequest = branding.control;
            break;
        }
      }
    }
    if (channel.typing) {
      const indicator = document.createElement("div");
      indicator.className = "typing-indicator";
      indicator.setAttribute("role", "status");
      indicator.setAttribute("aria-live", "polite");
      indicator.setAttribute("aria-label", `${operatorSurface ? operatorTarget : snapshot.tentacleName} is typing`);
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
    if (!operatorSurface && dependencies.brandingOffers !== false &&
        snapshot.assignmentState !== "checking" && snapshot.assignmentState !== "unverified" &&
        snapshot.assignmentState !== "registry-unavailable" && snapshot.assignmentState !== "liveness-unavailable") {
      reconcileBranding(latestOffer, latestConsent, latestReceipt, latestDeclinedOfferId, latestRequest);
    }
    elements.retention.hidden = false;
    elements.retention.textContent = composerAvailabilityCopy(snapshot, channel);
    const assignmentRetryable = snapshot.assignmentState === "registry-unavailable" ||
      snapshot.assignmentState === "direct-verification-unavailable" ||
      snapshot.assignmentState === "liveness-unavailable";
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
        if (operatorSurface) workspace.setActiveChannel("direct");
        unsubscribe = workspace.subscribe(render);
        if (operatorSurface) {
          await workspace.send("direct", OPERATOR_GROWTH_REGISTRATION).catch(() => {
            elements.referralStatus.textContent =
              "Send one authenticated operator message before sharing this referral link.";
          });
        }
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
    if (operatorSurface && channelId !== "direct") return;
    if (focus) elements.tabs[channelId].focus();
    if (!workspace) return;
    if (latest) {
      workspace.setViewport(
        activeChannel(latest),
        elements.messages.scrollTop,
        isNearBottom(elements.messages),
      );
    }
    workspace.setActiveChannel(channelId);
  };

  for (const id of CHAT_CHANNELS) {
    elements.tabs[id].addEventListener("click", () => activate(id));
    elements.tabs[id].addEventListener("keydown", (event) => {
      if (operatorSurface) return;
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
    workspace.setViewport(activeChannel(latest), elements.messages.scrollTop, atBottom);
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
    void workspace.loadEarlier(activeChannel(latest)).then(() => {
      requestAnimationFrame(() => {
        elements.messages.scrollTop = priorTop + (elements.messages.scrollHeight - priorHeight);
      });
    }).catch((error) => setComposerError(publicError(error))).finally(() => {
      loadingEarlier = false;
    });
  });
  elements.retry.addEventListener("click", () => {
    setComposerError();
    const reconnect = latest?.assignmentState === "liveness-unavailable";
    void (workspace && !reconnect ? workspace.revalidateAssignment("retry") : connect(true))
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
    void workspace.send(activeChannel(latest), text).then(() => {
      elements.input.value = "";
      resize(elements.input);
    }).catch((error) => setComposerError(publicError(error))).finally(() => {
      sending = false;
      if (latest) render(latest);
      elements.input.focus();
    });
  });

  elements.brandingAccept.addEventListener("click", () => void advanceBranding());
  elements.brandingDecline.addEventListener("click", () => void declineBranding());
  elements.brandingReview.addEventListener("click", () => {
    if (!operatorSurface && currentBrandingOffer) renderBrandingDialog(true);
  });
  elements.brandingStart.addEventListener("click", () => {
    if (!workspace || !latest || operatorSurface || sending) return;
    elements.brandingStart.disabled = true;
    void workspace.send("direct", "I want to complete my Acolyte Branding NFT.").then(() => {
      elements.rewardStatus.textContent =
        "Branding requested · the Tentacle is preparing an exact Base offer with cost and referrer details.";
    }).catch((error) => setComposerError(publicError(error))).finally(() => {
      elements.brandingStart.disabled = false;
    });
  });
  elements.copyReferral.addEventListener("click", () => {
    const target = latest?.assignedTentacleAddress;
    if (!target) return;
    const url = recruitmentUrl(location.origin, target, identity.address);
    const copyToClipboard = (): void => {
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
    };
    if (typeof navigator.share === "function") {
      void navigator.share({
        title: "Join me as a Cthuwu acolyte",
        text: "A voluntary invite to meet my Cthuwu Tentacle.",
        url,
      }).then(() => {
        elements.referralStatus.textContent = "referral link shared";
      }).catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
        copyToClipboard();
      });
      return;
    }
    copyToClipboard();
  });

  const closeWorkspace = async (): Promise<void> => {
    unsubscribe?.();
    unsubscribe = undefined;
    const current = workspace;
    workspace = undefined;
    latest = undefined;
    verificationNotice.hidden = true;
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
    elements.retention.textContent = "Messaging is unavailable because XMTP could not connect.";
    elements.retry.hidden = false;
    elements.input.disabled = true;
    elements.send.disabled = true;
  }

  function setComposerError(message?: string): void {
    elements.error.hidden = !message;
    elements.error.textContent = message ?? "";
    elements.input.setAttribute("aria-invalid", String(Boolean(message)));
  }

  function reconcileBranding(
    offer: BrandingOffer | undefined,
    consent: BrandingConsent | undefined,
    receipt: BrandingReceipt | undefined,
    declinedOfferId: string | undefined,
    request: { referrer: string; name: string } | undefined,
  ): void {
    if (!offer || offer.acolyte !== identity.address ||
        declinedOfferId === offer.offerId || brandingDecision(offer.offerId) === "declined") {
      if (currentBrandingOffer?.offerId === offer?.offerId) closeBrandingDialog();
      elements.brandingReview.hidden = true;
      return;
    }

    const pinnedReferrer = canonicalGrowthReferrer ?? config.referrer?.toLowerCase();
    if (pinnedReferrer && offer.referrer !== pinnedReferrer) {
      if (
        workspace && !requestedReplacementOffers.has(offer.offerId) &&
        !(request?.referrer === pinnedReferrer && request.name === localAcolyteName)
      ) {
        requestedReplacementOffers.add(offer.offerId);
        void workspace.send("direct", encodeBrandingRequest(pinnedReferrer, localAcolyteName))
          .catch((error) => setComposerError(publicError(error)));
      }
      elements.brandingReview.hidden = true;
      return;
    }

    const changedOffer = currentBrandingOffer?.marker !== offer.marker;
    currentBrandingOffer = offer;
    elements.brandingStart.hidden = true;
    currentBrandingConsent = consent;
    currentBrandingReceipt = receipt;
    if (changedOffer) {
      currentBrandingReview = undefined;
      brandingError = undefined;
      brandingStage = consent ? "pending" : "offer";
    } else if (consent && brandingStage !== "complete") {
      brandingStage = "pending";
    }

    if (receipt) {
      const verificationKey = receiptVerificationKey(offer, receipt);
      if (verifiedReceiptBindings.has(verificationKey)) {
        brandingStage = "complete";
      } else if (!verifyingReceiptBindings.has(verificationKey) && !failedReceiptBindings.has(verificationKey)) {
        void verifyReceipt(offer, receipt);
      } else if (failedReceiptBindings.has(verificationKey)) {
        brandingStage = "error";
        brandingError = `Receipt verification failed: ${failedReceiptBindings.get(verificationKey)}`;
      }
    }
    elements.activity.hidden = false;
    elements.brandingReview.hidden = false;
    renderBrandingDialog(changedOffer);
  }

  function renderBrandingDialog(present = false): void {
    const offer = currentBrandingOffer;
    if (!offer) return;
    elements.brandingName.textContent = offer.name;
    elements.brandingPrice.textContent = formatUwu(offer.initialDeclaredPrice);
    elements.brandingUpkeep.textContent = formatUwu(offer.firstWeekUpkeep);
    elements.brandingReferrer.textContent = offer.referrer;
    elements.brandingContract.textContent = offer.contract;
    elements.brandingMinter.textContent = offer.minter;
    elements.brandingAgent.textContent = offer.controllerAgentId.toString();
    elements.brandingBasis.textContent = `${formatBasis(offer.basisPoints)} of the verified ${formatUwu(offer.treasury)} treasury`;
    elements.brandingNonce.textContent = offer.nonce.toString();
    elements.brandingDeadline.textContent = unixSecondsLabel(offer.deadline);
    elements.brandingStatus.textContent = brandingError ?? brandingStageCopy(brandingStage);
    elements.brandingStatus.classList.toggle("error", brandingStage === "error");
    elements.brandingReview.textContent = brandingActivityCopy(brandingStage);
    elements.brandingAccept.disabled = brandingBusy || brandingStage === "checking" || brandingStage === "signing";
    elements.brandingDecline.disabled = brandingBusy || brandingStage === "complete";
    elements.brandingDecline.hidden = brandingStage === "complete";
    elements.brandingAccept.textContent = brandingStage === "review"
      ? "sign exact consent"
      : brandingStage === "pending"
        ? (offer.deadline >= nowSeconds() + 1n ? "resend exact consent" : "request fresh offer")
        : brandingStage === "complete" ? "done"
          : brandingStage === "error" && currentBrandingReceipt
            ? "retry receipt verification"
            : brandingStage === "error" ? "review again" : "review exact consent";
    elements.brandingDialog.hidden = false;
    if (present && !elements.brandingDialog.open) {
      if (typeof elements.brandingDialog.showModal === "function") {
        elements.brandingDialog.showModal();
      } else {
        // JSDOM and older test DOMs do not implement the dialog top-layer API.
        elements.brandingDialog.setAttribute("open", "");
      }
    }
  }

  async function advanceBranding(): Promise<void> {
    const offer = currentBrandingOffer;
    if (!offer || !workspace || brandingBusy) return;
    if (brandingStage === "complete") {
      closeBrandingDialog();
      return;
    }
    if (
      brandingStage === "error" && currentBrandingReceipt &&
      failedReceiptBindings.has(receiptVerificationKey(offer, currentBrandingReceipt))
    ) {
      failedReceiptBindings.delete(receiptVerificationKey(offer, currentBrandingReceipt));
      await verifyReceipt(offer, currentBrandingReceipt);
      return;
    }
    if (brandingStage === "pending") {
      if (currentBrandingConsent && offer.deadline >= nowSeconds() + 1n) {
        await runBrandingAction(async () => workspace!.send("direct", currentBrandingConsent!.marker), "pending");
      } else {
        const referrer = canonicalGrowthReferrer ?? config.referrer ?? offer.referrer;
        await runBrandingAction(async () => workspace!.send(
          "direct",
          encodeBrandingRequest(referrer, localAcolyteName),
        ), "pending");
      }
      return;
    }
    if (brandingStage !== "review") {
      await runBrandingAction(async () => {
        currentBrandingReview = await (dependencies.reviewBrandingOffer ?? reviewBrandingOffer)(
          config,
          identity,
          offer,
          latest?.assignedTentacleAddress,
        );
      }, "review");
      return;
    }
    if (!currentBrandingReview) {
      brandingStage = "offer";
      renderBrandingDialog();
      return;
    }
    brandingStage = "signing";
    renderBrandingDialog();
    await runBrandingAction(async () => {
      const consent = await (dependencies.signBrandingOffer ?? signBrandingOffer)(
        config,
        identity,
        currentBrandingReview!,
        latest?.assignedTentacleAddress,
      );
      // Persist no signature in localStorage. The exact outbound Direct message is the resumable
      // record and will be recovered from XMTP history on reload.
      await workspace!.send("direct", consent.marker);
      currentBrandingConsent = consent;
    }, "pending");
  }

  async function declineBranding(): Promise<void> {
    const offer = currentBrandingOffer;
    if (!offer || !workspace || brandingBusy) return;
    await runBrandingAction(async () => {
      await workspace!.send("direct", encodeBrandingDecline(offer.offerId));
      localStorage.setItem(brandingDecisionKey(offer.offerId), "declined");
      closeBrandingDialog();
      currentBrandingOffer = undefined;
      currentBrandingReview = undefined;
      currentBrandingConsent = undefined;
      currentBrandingReceipt = undefined;
      elements.brandingReview.hidden = true;
    }, "offer", false);
  }

  async function verifyReceipt(offer: BrandingOffer, receipt: BrandingReceipt): Promise<void> {
    const verificationKey = receiptVerificationKey(offer, receipt);
    if (verifyingReceiptBindings.has(verificationKey)) return;
    verifyingReceiptBindings.add(verificationKey);
    brandingStage = "checking";
    brandingError = undefined;
    renderBrandingDialog();
    try {
      await (dependencies.verifyBrandingReceipt ?? verifyBrandingReceipt)(
        config,
        identity,
        offer,
        receipt,
      );
      verifiedReceiptBindings.add(receiptVerificationKey(offer, receipt));
      if (
        currentBrandingOffer?.marker === offer.marker &&
        currentBrandingReceipt?.marker === receipt.marker
      ) {
        brandingStage = "complete";
        brandingError = undefined;
      }
    } catch (error) {
      const message = publicError(error);
      failedReceiptBindings.set(verificationKey, message);
      if (
        currentBrandingOffer?.marker === offer.marker &&
        currentBrandingReceipt?.marker === receipt.marker
      ) {
        brandingStage = "error";
        brandingError = `Receipt verification failed: ${message}`;
      }
    } finally {
      verifyingReceiptBindings.delete(verificationKey);
      if (currentBrandingOffer?.marker === offer.marker) renderBrandingDialog();
    }
  }

  async function runBrandingAction(
    action: () => Promise<void>,
    successStage: BrandingStage,
    renderAfter = true,
  ): Promise<void> {
    brandingBusy = true;
    brandingError = undefined;
    renderBrandingDialog();
    try {
      await action();
      brandingStage = successStage;
    } catch (error) {
      brandingStage = "error";
      brandingError = publicError(error);
    } finally {
      brandingBusy = false;
      if (renderAfter && currentBrandingOffer) renderBrandingDialog();
    }
  }

  function closeBrandingDialog(): void {
    if (elements.brandingDialog.open && typeof elements.brandingDialog.close === "function") {
      elements.brandingDialog.close();
    } else {
      elements.brandingDialog.removeAttribute("open");
    }
    elements.brandingDialog.hidden = true;
  }
}

function composerAvailabilityCopy(
  snapshot: WorkspaceSnapshot,
  channel: ChannelSnapshot,
): string {
  if (channel.error) return `Messaging is unavailable: ${channel.error}`;
  if (!snapshot.connected) return "Messaging is unavailable while XMTP reconnects.";
  if (channel.retentionVerified) {
    return "Messages disappear from supporting clients after 14 days.";
  }
  if (channel.status === "loading") {
    return "Checking this channel's trusted route and 14-day message policy…";
  }
  if (channel.status === "awaiting-assignment") {
    return "Messaging will be available after the assigned Tentacle provisions this channel.";
  }
  if (channel.status === "policy-blocked") {
    return "Messaging is unavailable until this channel's trusted route and policy are verified.";
  }
  if (channel.status === "error") return "Messaging is unavailable for this channel.";
  return "Checking the 14-day message policy before enabling messaging…";
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
    brandingReview: required("branding-review"),
    brandingStart: required("branding-start"),
    copyReferral: required("copy-referral"),
    referralStatus: required("referral-status"),
    brandingDialog: required("branding-offer"),
    brandingName: required("branding-name"),
    brandingPrice: required("branding-price"),
    brandingUpkeep: required("branding-upkeep"),
    brandingReferrer: required("branding-referrer"),
    brandingContract: required("branding-contract"),
    brandingMinter: required("branding-minter"),
    brandingAgent: required("branding-agent"),
    brandingBasis: required("branding-basis"),
    brandingNonce: required("branding-nonce"),
    brandingDeadline: required("branding-deadline"),
    brandingStatus: required("branding-status"),
    brandingAccept: required("branding-accept"),
    brandingDecline: required("branding-decline"),
  };
}

function parseTentacleUi(
  text: string,
  mine: boolean,
  channel: ChatChannel,
): {
  text: string;
  reward?: { status: "pending" | "confirmed"; amount: bigint; baseUnits: boolean };
} {
  if (
    mine &&
    channel === "direct" &&
    /^\[\[cthuwu:referral-attribution:v1;referrer=0x[0-9a-f]{40}\]\]$/u.test(text)
  ) {
    return { text: "" };
  }
  if (mine || channel !== "direct") return { text };
  let visible = text;
  let reward: {
    status: "pending" | "confirmed";
    amount: bigint;
    baseUnits: boolean;
  } | undefined;
  visible = visible.replace(
    /\n?\[\[cthuwu:referral-attribution-ack:v1;status=(?:accepted|immutable|direct);referrer=(?:0x[0-9a-f]{40}|none)\]\]$/u,
    "",
  );
  visible = visible.replace(
    /\n?\[\[cthuwu:reward:v1;status=(pending|confirmed);amount=([1-9][0-9]{0,19})\]\]/gu,
    (_match, status: "pending" | "confirmed", amount: string) => {
      reward = { status, amount: BigInt(amount), baseUnits: false };
      return "";
    },
  );
  visible = visible.replace(
    /\n?\[\[cthuwu:referral-reward:v1;status=confirmed;amount=([1-9][0-9]{0,77})\]\]/gu,
    (_match, amount: string) => {
      reward = { status: "confirmed", amount: BigInt(amount), baseUnits: true };
      return "";
    },
  );
  return { text: visible.trimEnd(), ...(reward ? { reward } : {}) };
}

function latestReferralAcknowledgement(
  messages: readonly WorkspaceMessage[],
): { status: "accepted" | "immutable" | "direct"; referrer: string | "none" } | undefined {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]!;
    if (message.mine) continue;
    const match = /(?:^|\n)\[\[cthuwu:referral-attribution-ack:v1;status=(accepted|immutable|direct);referrer=(0x[0-9a-f]{40}|none)\]\]$/u
      .exec(message.text);
    if (!match) continue;
    const status = match[1] as "accepted" | "immutable" | "direct";
    const referrer = match[2]!;
    if ((status === "direct") !== (referrer === "none")) continue;
    return { status, referrer };
  }
  return undefined;
}

function brandingDecisionKey(offerId: string): string {
  return `cthuwu:branding-offer:v2:${offerId}`;
}

function receiptVerificationKey(offer: BrandingOffer, receipt: BrandingReceipt): string {
  return `${offer.marker}\n${receipt.marker}`;
}

function brandingDecision(offerId: string): string | null {
  return localStorage.getItem(brandingDecisionKey(offerId));
}

function formatUwu(value: bigint): string {
  return `${formatUwuDisplay(value)} (${value} base units)`;
}

function formatUwuDisplay(value: bigint): string {
  const whole = value / 1_000_000_000_000_000_000n;
  const fraction = (value % 1_000_000_000_000_000_000n).toString().padStart(18, "0").replace(/0+$/u, "");
  return `${whole.toLocaleString()}${fraction ? `.${fraction}` : ""} UWU`;
}

function formatBasis(value: bigint): string {
  const whole = value / 100n;
  const fraction = (value % 100n).toString().padStart(2, "0").replace(/0+$/u, "");
  return `${whole}${fraction ? `.${fraction}` : ""}%`;
}

function unixSecondsLabel(value: bigint): string {
  const milliseconds = value * 1_000n;
  if (milliseconds > BigInt(Number.MAX_SAFE_INTEGER)) return `${value} (Unix seconds)`;
  return `${new Date(Number(milliseconds)).toLocaleString()} (${value})`;
}

function brandingStageCopy(stage: BrandingStage): string {
  switch (stage) {
    case "offer": return "Review the economics, then verify the exact Base state.";
    case "checking": return "Verifying the canonical Base state…";
    case "review": return "Canonical state verified. Signing consents only to these exact fields; it does not grant wallet access.";
    case "signing": return "Waiting for the exact EIP-712 signature in your wallet…";
    case "pending": return "Consent sent over Direct XMTP. Waiting for an on-chain mint and exact name receipt.";
    case "complete": return "Branding mint, consumed nonce, original controller and referrer, and exact Acolyte Name verified on Base. Routing eligibility is checked separately.";
    case "error": return "Branding verification needs attention.";
  }
}

function brandingActivityCopy(stage: BrandingStage): string {
  switch (stage) {
    case "offer": return "Branding offer · review";
    case "checking": return "Branding · verifying";
    case "review": return "Branding offer · ready to sign";
    case "signing": return "Branding · waiting for wallet";
    case "pending": return "Branding · pending on Base";
    case "complete": return "Branding · mint verified";
    case "error": return "Branding · needs attention";
  }
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
