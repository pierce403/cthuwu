# XMTP Rust integration notes

Last checked: 2026-08-01

XMTP's shared core implementation is [libxmtp](https://github.com/xmtp/libxmtp). The repository contains the `xmtp_mls`, API, cryptography, and protocol crates, and produces the WASM/N-API/FFI bindings used by platform SDKs.

For Cthuwu, direct Rust integration is attractive because it avoids embedding Node beside the Rust CLI. It also carries integration risk: libxmtp is primarily the shared core and its platform SDK bindings are the polished integrator surfaces.

## Implementation approach

1. Prototype against a pinned libxmtp commit, not an unbounded git branch.
2. Put every concrete XMTP type inside `transport::xmtp`.
3. Expose a narrow internal `Transport` trait using Cthuwu-owned message types.
4. Store the pinned commit and build prerequisites here.
5. Add an XMTP dev-network interoperability test before production use.
6. Track upstream schema/database migrations and test upgrade/rollback behavior.

Do not create a pretend crate dependency until the exact upstream revision and public API have been validated in a compiling prototype.
