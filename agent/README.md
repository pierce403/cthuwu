# Cthuwu XMTP sidecar

This private Node package gives the Rust `uwubot` process a supported XMTP
transport through `@xmtp/agent-sdk`. It is an implementation detail of the one
operator-facing command, not a second bot.

## Build and verify

Node 22 or newer is required.

```sh
npm ci
npm run typecheck
npm test
npm run build
```

## Local state

On first launch the sidecar creates a dedicated secp256k1 wallet key and a
32-byte XMTP database-encryption key. It writes them atomically to
`$UWUBOT_DATA_DIR/state/xmtp-identity.json` with owner-only permissions and
reuses them on later launches. The XMTP database defaults to the
environment-specific `$UWUBOT_DATA_DIR/state/xmtp/<environment>/` directory.

`XMTP_ENV` must be `local`, `dev`, or `production`. An identity or database
directory created for one environment is rejected in another. Operators may
override `XMTP_DB_DIRECTORY`, but private wallet and database keys are never
accepted through environment variables. The sidecar reads them directly from
the owner-only persisted identity and constructs `Agent.create(...)` options in
memory; it does not use `Agent.createFromEnv`.

The identity file contains the wallet private key in plaintext protected by
filesystem permissions. Keep the data directory private, backed up securely,
and out of source control. Neither secret is written to logs, process arguments,
protocol frames, or the process environment.

## JSONL boundary

Standard output contains protocol frames only. All operational diagnostics go
to standard error without message bodies or secrets.

The sidecar measures the UTF-8 byte length before admitting an inbound one-to-one text DM. For a
message of at most 16 KiB admitted to the bridge's normal pending set, it writes:

```json
{"type":"inbound_text","id":"request-id","messageId":"xmtp-message-id","senderInboxId":"sender-inbox-id","sentAtNs":"1750000000000000000","deadlineUnixMs":1750000300000,"conversationId":"conversation-id","text":"hello"}
```

For a message larger than 16 KiB, the sidecar does not place the original message text on the JSONL
boundary. With a normal pending slot available, it sends this metadata-only control frame (the
shared schema's `text` field is present but empty):

```json
{"type":"reject_oversized","id":"rejection-id","messageId":"xmtp-message-id","senderInboxId":"sender-inbox-id","sentAtNs":"1750000000000000000","deadlineUnixMs":1750000300000,"conversationId":"conversation-id","text":""}
```

Rust validates the frame, classifies the authenticated sender, and attempts the durable
processed-message claim before responding. The first claim gets a role-specific too-large `reply`;
a repeated delivery gets `ignore`. Rust never opens contact state or dispatches the original
content, a model, or a tool for `reject_oversized`.

If the bridge's bounded pending set is already full, it sends one bounded durability handshake
instead of silently dropping or locally answering the XMTP message. This control frame also carries
no message text:

```json
{"type":"reject_inbound","id":"rejection-id","messageId":"xmtp-message-id","senderInboxId":"sender-inbox-id","sentAtNs":"1750000000000000000","deadlineUnixMs":1750000300000,"conversationId":"conversation-id","text":""}
```

Rust validates the metadata, pins the authenticated role, and attempts the same durable
processed-message claim used by normal delivery. It does not call a model, open contact state, or
invoke a tool for `reject_inbound`. If this is the first durable claim for that XMTP message ID, Rust
returns one busy `reply`; a repeated delivery returns `ignore`. The bridge waits for this
acknowledgement before releasing its single rejection slot, so overload handling is bounded as well.
If an oversized-message rejection reaches an already-full pending set, this empty-text
`reject_inbound` handshake is the bounded fallback; the original content still does not cross.

Normal `inbound_text` also gets its durable claim before authority-lane admission. If its public or
operator lane is already occupied, Rust returns a busy `reply` without dispatching content. Every
rejection path makes the claim an at-most-once tombstone: replaying the same XMTP message ID produces
`ignore`. A person or client that still wants the work performed must retry by sending a **new XMTP
message**, with a new message ID, after capacity becomes available; an oversized message must also be
shortened.

`uwubot` must answer within 300 seconds (configurable from 2–300 seconds with
`UWUBOT_REPLY_TIMEOUT_MS`) using exactly one of:

```jsonl
{"type":"reply","id":"request-id","text":"hello back"}
{"type":"ignore","id":"request-id"}
```

Unknown, late, and malformed lines are ignored. A matching malformed or
oversized reply fails its request immediately instead of waiting for the
timeout. `deadlineUnixMs` is created locally by the bridge from that same timeout; Rust reserves
time to return a deadline result, authenticates the role, caps public work at 120 seconds, and
cancels active work before the bridge drops the pending ID. Provider attempts derive their own
smaller budgets from that authenticated deadline. Group messages and non-text content never cross
this boundary.

## Three-channel control plane

The sidecar registers `cthuwu.app/join:1.0` and `cthuwu.app/assignment:1.0` custom content codecs.
They have no text fallback and do not request push delivery. The first Agent middleware consumes
both types before ordinary events. A join is accepted only in a DM after a network-fresh lookup
binds the envelope's `senderInboxId` to exactly one current Ethereum identifier; no identity or
routing claim is accepted from the payload. Group messages, including forged control content, never
enter the Rust, inference, or contact path.

Owner-only `state/xmtp-chat-control.json` binds the Tentacle to exactly one Acolytes group and the
configured Global group. Both groups use strict versioned `appData`, admin-only permissions, and
XMTP disappearing settings of `fromNs = 1` and `inNs = 1209600000000000` (14 days). Normal
enrollment never creates a Global group.

After the Tentacle has an active, locally verified ERC-8004 identity, create the production Global
group once:

```sh
CTHUWU_GLOBAL_ADMIN_INBOX_IDS=<other-tentacle-inbox,...> \
  uwubot chat global create
```

Persist the returned ID as `CTHUWU_GLOBAL_GROUP_ID` on every authorized Tentacle. Inspect the exact
group and reconcile missing configured members/admins with:

```sh
CTHUWU_GLOBAL_GROUP_ID=<group-id> \
CTHUWU_GLOBAL_ADMIN_INBOX_IDS=<other-tentacle-inbox,...> \
  uwubot chat global inspect
```

Unexpected admins, wrong `appData`, wrong environment, non-admin-only policy, a conflicting
persisted ID, or a second create request fails closed. `CTHUWU_BRANDING_CONTRACT` defaults to the
pinned canonical Base deployment, and any explicit override must equal that address. A Base,
Branding, registry, profile, or endpoint validation failure freezes enrollment and pruning rather
than becoming abandonment.
