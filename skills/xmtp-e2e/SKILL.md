---
name: xmtp-e2e
description: Verify Cthuwu browser-to-runtime XMTP messaging and persistence.
---

# XMTP end-to-end verification

## Preconditions

- Use dedicated test identities with no material funds.
- Confirm browser and runtime use the same explicit XMTP environment.
- Confirm logs do not include private keys or plaintext message bodies.
- Record dependency versions and the runtime commit.

## Procedure

1. Start with clean, environment-specific test data directories.
2. Run `cthuwu doctor`; resolve storage, network, and model failures.
3. Run `cthuwu serve` with the deterministic echo model.
4. Build and serve `web/dist` over localhost.
5. Connect the browser and record both inbox identifiers.
6. Send a unique nonce as a text DM and verify exactly one echo response.
7. Restart the runtime and browser, then verify identity and history persistence.
8. Resend or replay the inbound message and verify it is not answered twice.
9. Interrupt the network, recover it, and verify processing resumes.
10. Record sanitized results in `docs/test-runs/`.

## Pass criteria

- One inbound message produces exactly one response.
- Both identities and conversation history persist across restart.
- No secret or message body appears in normal logs.
- A failure is surfaced with an actionable error rather than silent loss.
