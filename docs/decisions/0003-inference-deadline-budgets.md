# 0003: Budget inference from the authenticated request deadline

Date: 2026-08-05

## Status

Accepted.

## Context

The XMTP bridge previously allowed 90 seconds for a request, while each OpenAI-compatible HTTP call
could consume 45 seconds and Ollama could consume 75. Venice can require catalog validation, TEE
attestation, completion, tool continuation, and policy repair in one route. Treating each request's
static timeout as the policy left no dependable time for local fallback.

Comparable agent harnesses provide useful pieces but no complete fallback budget:

- [Pi](https://pi.dev/docs/latest/settings) keeps provider retries at zero, uses abortable outer
  retries, separates response-header and body-idle timers, and refreshes its persisted catalog on an
  independent schedule. It has no absolute turn deadline or cross-provider fallback reserve.
- [Hermes](https://github.com/NousResearch/hermes-agent/blob/01a1037d1e6d7b6eb96a786ef282c3aea4818194/website/docs/user-guide/configuration.md#L966-L981)
  distinguishes request, socket-read, and stale-stream timeouts and lets an outer loop own fallback,
  but its defaults are much longer and it does not subtract fallback time from a shared deadline.
- [OpenCode](https://github.com/anomalyco/opencode/blob/24470e52a537f7a4d08be681b95b3a1891fc1bfa/packages/opencode/src/provider/provider.ts#L1737-L1767)
  composes caller cancellation with full, header, and stream-idle signals, but does not provide a
  provider-fallback chain.
- [Goose](https://github.com/block/goose/blob/1c1bd5299a243f309cb251d2bbe429c7f470793e/crates/goose-providers/src/ollama.rs#L467-L513)
  separates Ollama time-to-first-token from inter-token stalls, but has no runtime provider failover.

## Decision

The locally generated, authenticated XMTP deadline is the authority ceiling. Node remains role
agnostic and supplies a 300-second default envelope. Rust keeps one second for returning the XMTP
response, so an operator route can use at most 299 seconds. After Rust authenticates and pins the
role, it caps public work at 120 seconds and allows operator work to use the remaining envelope.

Before each provider candidate, the router computes:

```text
candidate budget = min(provider cap, remaining authenticated time - later fallback reserve)
```

Public remote candidates are capped at 30 seconds. Operator Venice uses
`UWUBOT_VENICE_TIMEOUT_SECONDS`, defaulting to 120 seconds. An operator remote route reserves two
capped local model phases (up to the 75-second safety cap, or a smaller configured Ollama timeout,
each), one model-selected tool phase of up to 30 seconds, and one second for deterministic fallback. The default reserve is thus
181 seconds, leaving an effective Venice maximum of about 118 seconds inside the default 299-second
operator budget even though its configured cap is 120 seconds. Model-selected operator tools are
likewise limited by the remaining deadline minus a final local-completion reserve.

A candidate receives one absolute deadline across catalog validation, attestation, completion,
search/tool continuation, and policy repair. Each HTTP request is clamped again to the remaining
candidate time. Budget-skipped providers are not marked unhealthy. Failure cooldown is lane-aware,
so a short public-lane failure does not suppress a longer operator attempt at that provider.

Public completions request at most 300 output tokens. Search remains available only through the
closed public adapter, and the runtime exposes its schema only when the current message explicitly
asks for current or web-verifiable information. Policy-repair completions expose no tools.

Venice catalog capabilities and TEE attestation use independent cache timestamps. Catalog success is
valid for four hours and is recorded before attestation; attestation remains valid for five minutes.
The shared validation refresh is coalesced under a mutex, but acquiring that mutex is itself bounded
by the current candidate deadline.

Timeout logs name the provider lane and phase without recording prompts. Named phases include model
catalog, validation-lock wait, TEE attestation, chat completion, tool continuation, policy repair,
and web search.

## Consequences

A Venice stall fails over while Ollama still has a meaningful execution window. Public chatter fails
remote work promptly, while authenticated operator/tool work can wait longer. Raising a provider cap
cannot silently consume fallback time because the authenticated envelope and reserve arithmetic still
win. A slow model-selected operator tool cannot consume the local final-completion window.

The current non-streaming adapter has a whole-request timer rather than separate TTFB and inter-token
stall timers. If streaming is introduced, those transport timers must remain subordinate to the same
absolute candidate deadline.
