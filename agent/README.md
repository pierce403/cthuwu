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
provide `XMTP_WALLET_KEY`, `XMTP_DB_ENCRYPTION_KEY`, or `XMTP_DB_DIRECTORY`;
the persisted identity must agree with any supplied keys.

The identity file contains the wallet private key in plaintext protected by
filesystem permissions. Keep the data directory private, backed up securely,
and out of source control. Neither secret is written to logs.

## JSONL boundary

Standard output contains protocol frames only. All operational diagnostics go
to standard error without message bodies or secrets.

For each inbound one-to-one text DM, the sidecar writes:

```json
{"type":"inbound_text","id":"request-id","messageId":"xmtp-message-id","senderInboxId":"sender-inbox-id","conversationId":"conversation-id","text":"hello"}
```

`uwubot` must answer within 90 seconds (configurable from 1–300 seconds with
`UWUBOT_REPLY_TIMEOUT_MS`) using exactly one of:

```json
{"type":"reply","id":"request-id","text":"hello back"}
{"type":"ignore","id":"request-id"}
```

Unknown, late, and malformed lines are ignored. A matching malformed or
oversized reply fails its request immediately instead of waiting for the
timeout. Group messages and non-text content never cross this boundary.
