# XMTP Rust integration notes

Last checked: 2026-08-01

XMTP's shared core implementation is [libxmtp](https://github.com/xmtp/libxmtp). The repository contains the `xmtp_mls`, API, cryptography, and protocol crates, and produces the WASM/N-API/FFI bindings used by platform SDKs.

For Cthuwu, direct Rust integration is attractive because it avoids embedding Node beside the Rust CLI. It also carries integration risk: libxmtp is primarily the shared core and its platform SDK bindings are the polished integrator surfaces.

## Current decision

The first release uses `@xmtp/agent-sdk@2.3.0` behind the Rust process boundary described in [decision 0002](../decisions/0002-agent-sdk-sidecar.md). This is XMTP's supported bot integration surface and already handles content decoding, self-message filtering, and stream recovery.

A native implementation was source-validated against libxmtp revision `66944e28f1d19269be7af0e11e165492f61a2b19` on 2026-08-01. It is technically possible, but every `xmtp_*` crate would need to be pinned to the same Git revision along with libxmtp's Diesel and hpke-rs patches. The workspace requires Rust 1.94 or newer and does not publish those crates as a supported public SDK.

An important interoperability pitfall: native `StoredGroupMessage.decrypted_message_bytes` contains protobuf `EncodedContent`, not raw UTF-8. Browser/Node text must be decoded and encoded through `xmtp_content_types::text::TextCodec`. The `xdbg` utility's raw-byte paths are not a suitable application transport.

Reconsider direct Rust only when a stable upstream surface materially reduces this maintenance cost. Keep any future concrete XMTP types inside the transport module and rerun database upgrade, replay, and browser interoperability tests.
