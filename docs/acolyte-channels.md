# Acolyte XMTP channels

This document specifies the in-progress three-channel browser workspace and Tentacle enrollment
protocol. It is separate from the Council control plane described in `docs/protocol/`: these groups
carry acolyte conversation and enrollment traffic, not Council routing, governance, or leases.

The canonical Acolyte Branding is funded and independently verified on Base, and the repository has
typed consent/mint/name-repair and assignment implementations. There is still no configured
production Global group or passing funded browser-wallet/XMTP end-to-end gate. Repository code and
local tests therefore do **not** establish production wallet or cross-Tentacle group
interoperability; the configured intro Tentacle remains the continuity path for its documented
fallback states.

## Browser workspace

The browser recovers one existing `StoredIdentity` and creates one `@xmtp/browser-sdk` `Client`.
That identity and client back exactly three product channels:

| Channel | Trusted binding | Purpose |
|---|---|---|
| Direct | Exact DM conversation ID with the assigned Tentacle inbox | Private acolyte-to-Tentacle conversation and versioned enrollment control. |
| Acolytes | Exact group ID returned by the assigned Tentacle | The assigned Tentacle and only its currently assigned acolytes. |
| Global | `readConversationIds[]` plus one `writeConversationId` | Cthuwu-wide acolyte conversation without coupling the UI to one physical group forever. |

The product does not expose arbitrary inbox lookup, arbitrary group creation, or a generic XMTP
conversation list in version 1. Accessible tabs, arrow-key navigation, unread counts, channel-local
pagination, scroll/read state, and distinct empty/loading/error states sit over these trusted
bindings. One shared composer always sends through the selected channel's exact current
conversation ID.

The normalized message shape includes `conversationId`, `senderInboxId`, `sentAtNs`, content type,
text, and `mine`. The client consumes all-message, new-group, and deleted-message streams, but it
admits only exact currently trusted conversation IDs into the workspace. Enrollment control content
is never rendered. A deletion event removes the matching message from the rendered channel.

Browser state uses only `cthuwu.chat.*` local-storage keys. Those keys must remain disjoint from the
leaderboard's `cthuwu:leaderboard:v1` cache and must not copy XMTP message bodies into another
application cache.

## Canonical assignment

The acolyte address comes only from the recovered `StoredIdentity`. A DOM field, query parameter,
message payload, Agent0 record, leaderboard cache entry, or claimed address is never assignment
authority.

When a Branding contract is configured, one assignment snapshot is evaluated at one explicit Base
block. The browser must revalidate all of these facts at that same block:

1. the acolyte's Branding status and exact controller agent ID;
2. the Branding owner/controller wallet binding;
3. the canonical Identity Registry deployment and version;
4. `getAgentWallet` and current owner-or-authorized control for that exact agent ID;
5. byte-exact `cthuwu.allegiance = uwu-tentacle-v1`;
6. byte-exact `cthuwu.protocol = 1`; and
7. that exact agent's current on-chain ERC-8004 registration resolves to the production XMTP
   endpoint being selected.

Endpoint resolution is nested and fail-closed. The block-pinned `tokenURI(agentId)` must be a
bounded active `registration-v1` data URI with exactly one matching
`eip155:8453:0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` registration, exactly one
`CTHUWU-XMTP` version-`1` service whose endpoint is `xmtp://<64-lowercase-hex-inbox-id>`, and exactly
one `CTHUWU` service containing the nested bounded manifest. That manifest has exactly
`schemaVersion`, `protocol`, `tentacleId`, `erc8004`, `xmtp`, and `capabilities`; it must bind schema
and protocol `1`, the same Base registry and agent ID, XMTP `production`, the same outer endpoint,
and a bounded capability list containing `direct-xmtp-messaging`. A top-level endpoint without this
nested CTHUWU production binding is unavailable, not a routing result.

Active Branding remains canonical Base routing authority. Explicit `#t=` links use complete pinned,
indexing-error-free Agent0 discovery followed by canonical Base authorization. First-connect
Unminted selection starts from the rank-ordered, completely validated leaderboard snapshot; cached
data narrows at most five candidates but never authorizes the winning route. XMTP independently
binds every candidate inbox to its wallet, and the winner receives a fresh same-block canonical
registry/registration verification before Direct opens. The Direct display name also comes only
from that freshly verified bounded registration profile, with `Tentacle #<agentId>` as the fallback;
the cached name remains a discovery hint.

Assignment outcomes are intentionally asymmetric:

- `NotConfigured` is retained only for injected compatibility configurations. Normal production
  configuration always selects the pinned canonical Branding deployment.
- `Unminted` without an explicit verified `#t=` or retained eligible winner races non-push liveness
  queries against at most the top five funded profile-active, protocol-1, single-agent wallets in
  the last completely validated leaderboard snapshot. When absent, the browser first obtains one
  complete pinned, indexing-error-free Agent0 plus same-block Base UWU snapshot. The first exact
  authenticated response within 15 seconds wins only after fresh canonical Branding, registry,
  wallet/authorization, allegiance/protocol, and production agentURI endpoint verification. The
  route is persisted only after the Direct handoff and liveness-bound join publish succeed. No
  response is explicit and never selects the unprobed intro Tentacle; deliberate Retry opens a fresh
  workspace for one new bounded race.
- An explicit `#t=` wallet selects its sole active protocol-1 identity from that same complete Agent0
  directory. Multiple eligible identities remain ambiguous and fail closed. Before Direct opens,
  the selected identity receives the same fresh same-block canonical Base verification as a liveness
  winner; the explicit route does not send or require a liveness probe. For a fresh Unminted
  Acolyte this deliberate route is Direct-only: it sends no enrollment control, and the Acolytes and
  Global tabs report the policy explicitly instead of waiting for an assignment that cannot arrive.
- `Expired` or positively verified `Ineligible` continues to assign the intro Tentacle.
- `RegistryUnavailable`, a malformed canonical response, a block-consistency failure, or an
  unverifiable endpoint freezes Branding-based routing and presents a retryable state. It is never
  reinterpreted as abandonment, ineligibility, or permission to fall back.
- Only a fully verified `Active` Branding selects its exact controller Tentacle.

This bounded Direct probe proves that the responding XMTP sidecar is currently reachable; it is not
a Council heartbeat, long-term availability promise, or new central coordinator. One workspace
attempts it only during first connection (or the first explicit retry after a pre-probe failure),
never on periodic/resume refresh. Operators should still confirm `/registry-allegiance off` before
planned shutdown so an offline Tentacle leaves discovery.

Assignment is revalidated on connect, PWA resume, and a bounded periodic interval. A controller
change advances the assignment revision, stops sends through the old Direct and Acolytes bindings,
and enrolls with the newly assigned Tentacle. Global remains bound. Old conversation IDs are no
longer trusted workspace routes, while the former Tentacle's reconciliation removes the acolyte
from its old group.

No inbox ID, group ID, assignment revision, or conversation data belongs on-chain.

## Enrollment control messages

The pinned browser and Node SDKs register the assignment, typing, and liveness custom content types.
They have empty parameters, no
fallback text, no compression, and no push notification:

| Logical type | `authorityId` | `typeId` | Version |
|---|---|---|---|
| `cthuwu.join.v1` | `cthuwu.app` | `join` | `1.0` |
| `cthuwu.assignment.v1` | `cthuwu.app` | `assignment` | `1.0` |
| `cthuwu.liveness-query.v1` | `cthuwu.app` | `liveness-query` | `1.0` |
| `cthuwu.liveness-response.v1` | `cthuwu.app` | `liveness-response` | `1.0` |
| `cthuwu.liveness-join.v1` | `cthuwu.app` | `liveness-join` | `1.0` |

The exact join payload keys are:

```json
{
  "type": "cthuwu.join.v1",
  "requestId": "<32 lowercase hex characters>",
  "environment": "production"
}
```

Fresh non-intro Unminted enrollment uses a separate exact join bound to the winning query:

```json
{
  "type": "cthuwu.liveness-join.v1",
  "requestId": "<new 32 lowercase hex characters>",
  "environment": "production",
  "livenessRequestId": "<winning 32-lowercase-hex query ID>"
}
```

The query and response payloads bind the visible challenge, production environment, target agent,
and bounded deadline:

```json
{
  "type": "cthuwu.liveness-query.v1",
  "requestId": "<32 lowercase hex characters>",
  "environment": "production",
  "phrase": "fhtagn?",
  "expiresAtNs": "<positive decimal uint64>",
  "targetAgentId": "<canonical decimal uint256>"
}
```

```json
{
  "type": "cthuwu.liveness-response.v1",
  "requestId": "<the query requestId>",
  "environment": "production",
  "phrase": "fhtagn!",
  "tentacleAgentId": "<the target agent ID>"
}
```

The exact assignment payload keys are:

```json
{
  "type": "cthuwu.assignment.v1",
  "requestId": "<the join requestId>",
  "environment": "production",
  "revision": "<Base block number>:0x<64 lowercase hex block hash>",
  "tentacleAgentId": "<canonical decimal uint256>",
  "tentacleInboxId": "<64 lowercase hex characters>",
  "acolytesGroupId": "<64 lowercase hex characters>",
  "global": {
    "logicalChannelId": "cthuwu.global.v1",
    "readConversationIds": ["<trusted 64-lowercase-hex group ID>"],
    "writeConversationId": "<one ID also present in readConversationIds>",
    "adminInboxIds": ["<trusted 64-lowercase-hex inbox ID>"]
  },
  "retention": {
    "fromNs": "1",
    "inNs": "1209600000000000"
  }
}
```

Every payload is canonical UTF-8 JSON capped at 8 KiB with an exact key set. Global
read-conversation and admin-inbox arrays are nonempty, unique, and capped at 32 entries; the write
ID must appear in the read set. Unknown keys, versions, environments, type metadata, or fallback
content fail closed.

The assignment `revision` is the Tentacle's own canonical Base block sample. Its grammar is strict,
but the browser must not require it to equal the browser's independently sampled, earlier routing
block; the matching request ID and authenticated Direct envelope bind the response instead.

`agent/src/index.ts` intercepts every control before ordinary message forwarding. They must never
reach Rust, inference, contact memory, onboarding, token-tier conversation handling, or ordinary
chat history. Authentication comes from the XMTP envelope's `senderInboxId` and the sender identity
resolved by the SDK. Any address, inbox, Tentacle ID, group ID, or assignment claim inside the
payload is untrusted and cannot authenticate the request.

Liveness admission is process-local and deliberately short lived. It accepts only Direct queries
for the locally verified production agent, rejects expired or more-than-60-second horizons, tracks
request replays for 60 seconds, permits at most eight accepted probes per sender and 256 globally
per minute, and keeps bounded sender/replay/grant maps. A valid response creates one grant bound to
the authenticated sender inbox, its freshly resolved EOA, the query ID, and the local agent ID for
60 seconds from issuance so the browser can finish its independent Base and Direct checks. A send
failure revokes it. Successful enrollment persistence consumes it; ordinary join cannot claim the
first grant, while either exact join form may recover an already persisted matching enrollment when
an Assignment response was lost.

Normal group chatter is also terminal at the group transport/UI path in version 1. It never enters
the personal DM inference or memory path.

## Tentacle provisioning

For an authenticated, canonically assigned join request, the assigned Tentacle performs one
idempotent enrollment transaction:

1. load or create exactly one persisted Acolytes group for that Tentacle and environment;
2. ensure the authenticated acolyte inbox is a member;
3. ensure it is a member of the configured singleton Global group;
4. repair the required disappearing-message policy when authorized; and
5. return the exact trusted bindings and current assignment revision.

Retries, restarts, and duplicate control messages must return the same logical groups instead of
creating competitors. The environment/Tentacle-bound group IDs and authenticated enrollments live
in owner-only `state/xmtp-chat-control.json`. A bounded reconciliation periodically revalidates
current Branding assignments, removes acolytes that moved to another Tentacle, and rejects or
removes unexplained non-admin Acolytes membership rather than adopting it.

Global is an explicit production singleton. An authorized bootstrap/admin operation creates or
inspects it and grants the configured Tentacle inboxes admin status. Ordinary enrollment must fail
closed when that group is absent or invalid; it must never silently create another group named
"Global." Human-readable names are presentation only.

The supported one-shot admin commands are:

```bash
./uwu.sh chat global create
./uwu.sh chat global inspect
```

`create` requires that neither configuration nor persisted state already names a Global group. It
creates once, or recovers one exact self-created group from the crash window between XMTP creation
and local persistence, and prints the exact group ID; set that reviewed value as
`CTHUWU_GLOBAL_GROUP_ID` before normal service. A drifted self-created candidate blocks replacement
and requires repair/inspect rather than permitting a competitor. `inspect` requires the configured
ID and reconciles missing configured admin membership/elevation while rejecting unexpected elevated
admins or an invalid group. Both commands exit after the admin operation. Ordinary service startup
and enrollment never create Global.

A trusted group must match all of the following: exact conversation ID, XMTP environment, supported
versioned `appData`, logical channel kind, expected admins, and current membership/assignment data.
A matching name or attacker-supplied `appData` alone proves nothing.

The exact version-1 Acolytes `appData` object is:

```json
{
  "app": "cthuwu.chat",
  "version": 1,
  "environment": "production",
  "channel": "acolytes",
  "tentacleAgentId": "<canonical decimal uint256>",
  "tentacleInboxId": "<64 lowercase hex characters>"
}
```

The exact initial Global `appData` object is:

```json
{
  "app": "cthuwu.chat",
  "version": 1,
  "environment": "production",
  "channel": "global",
  "logicalChannelId": "cthuwu.global.v1",
  "shardId": "primary"
}
```

Extra/missing keys, a different environment/version/channel, a mismatched Tentacle binding, or a
noncanonical serialization fails validation. Acolytes requires the assigned Tentacle as creator,
adder, sole super-admin, and member with the pinned admin-only policy. Global requires the exact
configured elevated-admin set, those admins as members, the pinned admin-only policy, and the exact
configured conversation ID. The authenticated acolyte and assigned Tentacle must both be members of
each rendered group.

The [official XMTP conversation documentation](https://docs.xmtp.org/chat-apps/core-messaging/create-conversations#create-a-new-group-chat)
states that the maximum group chat size is 250 members. Global is therefore represented in the
browser as `readConversationIds[]` plus `writeConversationId` even while the first production
deployment uses one physical group. A future sharding design can add read groups and rotate the
write group without changing the three-tab UI or message model.

## Fourteen-day disappearing policy

Every trusted Direct, Acolytes, and Global conversation uses the XMTP disappearing-message policy:

```text
fromNs = 1n
inNs = 1_209_600_000_000_000n
```

Either Direct participant may repair the DM setting. A group admin sets or repairs it for Acolytes
and Global. Before enabling the composer for a channel, the browser verifies the exact policy and
shows: "messages disappear from supporting clients after 14 days." This is an XMTP client policy,
not a claim that all previously copied, exported, screenshotted, or independently stored data is
erased. Deleted-message stream events remove expired messages from the rendered workspace.

## Configuration

These names describe the production trust inputs. Branding defaults to the verified canonical Base
deployment; the Global group remains explicit and must never be inferred:

| Name | Consumer | Meaning |
|---|---|---|
| `VITE_CTHUWU_BASE_RPC_ENDPOINT` | Static browser build | Public read-only Base RPC; defaults to the checked-in origin-restricted Infura endpoint. |
| `VITE_CTHUWU_BRANDING_CONTRACT` | Static browser build | Verified Branding deployment; defaults to the canonical Base address. |
| `VITE_CTHUWU_ASSIGNMENT_REFRESH_MS` | Static browser build | Browser revalidation cadence; defaults to `600000`, with accepted values from `60000` through `3600000`. |
| `CTHUWU_RPC_ENDPOINT` | Tentacle runtime | Credential-free HTTPS or loopback HTTP Base RPC; defaults to `https://mainnet.base.org`. |
| `CTHUWU_BRANDING_CONTRACT` | Tentacle runtime | The same deployment used to authorize and reconcile enrollment; defaults to the canonical Base address. |
| `CTHUWU_GLOBAL_GROUP_ID` | Tentacle runtime | Exact singleton production Global conversation ID. |
| `CTHUWU_GLOBAL_ADMIN_INBOX_IDS` | Tentacle runtime/bootstrap | Comma-separated authorized Tentacle inbox-admin set, at most 32 including the local inbox. |
| `CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS` | Tentacle runtime | Membership sweep cadence; defaults to `900`, with accepted values from `60` through `86400`. |

For example, a reviewed static build configuration has this shape:

```dotenv
VITE_CTHUWU_BASE_RPC_ENDPOINT=https://base-mainnet.infura.io/v3/e1656809acaa4db18ea2ea40e489c4c8
VITE_CTHUWU_BRANDING_CONTRACT=0xd8c36f13d79a505c7fbdc5f6467ea3cd75e896da
VITE_CTHUWU_ASSIGNMENT_REFRESH_MS=600000
```

The matching Tentacle environment has this shape:

```dotenv
CTHUWU_RPC_ENDPOINT=https://mainnet.base.org
CTHUWU_BRANDING_CONTRACT=0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da
CTHUWU_GLOBAL_GROUP_ID=<64-lowercase-hex-group-id>
CTHUWU_GLOBAL_ADMIN_INBOX_IDS=<64-lowercase-hex-inbox-id>,<64-lowercase-hex-inbox-id>
CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS=900
```

`CTHUWU_GLOBAL_GROUP_ID` is required to enable authenticated three-channel enrollment. The local
Tentacle inbox is included in the admin set even when omitted from
`CTHUWU_GLOBAL_ADMIN_INBOX_IDS`. If enrollment configuration or the local verified ERC-8004
registration is absent, the sidecar leaves the existing Direct message path available rather than
creating a group implicitly.

The browser millisecond cadence and Tentacle second cadence are intentionally separate runtime
settings; neither configures the other.

Production configuration must bind the browser, Tentacles, Base chain, XMTP `production`
environment, Branding deployment, and Global group to one reviewed release. Do not infer values
from a group name, cached leaderboard row, or arbitrary Agent0 result.

## Test and release gates

Local tests must cover:

- idempotent join processing and exactly-one Acolytes group creation;
- forged control payloads and spoofed group names, `appData`, admins, members, and IDs;
- Branding reassignment, Direct/Acolytes handoff, and removal from the old group;
- exact send/receive routing across all three tabs;
- unread counts, pagination, scroll restoration, reconnect, and PWA resume;
- exact 14-day policy enforcement, repair authorization, composer gating, and deletion events;
- a hard assertion that group chatter and enrollment control never reach personal inference or
  contact memory;
- keyboard, mobile, desktop, and accessibility behavior; and
- preservation of the leaderboard, PWA lifecycle, identity backup/recovery, and cache namespace.

Production remains incomplete until a real production XMTP gate proves fresh enrollment,
idempotent reconnect, all three conversation bindings, admin policy, reassignment, retention, and
cross-Tentacle interoperability against the verified deployed Branding and singleton Global group.
