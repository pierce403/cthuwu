---
name: repository-validation
description: Run Cthuwu's compiled, manifest-pinned test and build profiles and report partial results truthfully.
---

# Repository validation

Use typed `test` or `build`; neither accepts a command string.

- `{"operation":"test","profile":"focused"}` runs Rust formatting and workspace tests.
- `{"operation":"test","profile":"required"}` adds Rust clippy, agent/web typechecks and tests,
  launcher/installer tests, and Foundry formatting/lint/tests.
- `{"operation":"build","profile":"runtime"}` initializes locked submodules, installs locked agent
  dependencies, builds the agent, and builds the Rust release.
- `{"operation":"build","profile":"required"}` adds the locked web install/build, launcher
  production smoke test, and Foundry size build.

`update` and `pr` use the validation IDs embedded from `repository-maintenance.json` and resolved
through a compiled command allowlist. Workspace text cannot add a command. Every child has bounded
output, a per-command timeout, and the overall authenticated maintenance deadline. A cold run may
need separate `test` and `build` messages; report completed, failed, skipped, and timed-out steps
exactly. Never describe partial validation as a pass.

Validation children never inherit `GH_TOKEN`, `GITHUB_TOKEN`, `GH_CONFIG_DIR`, or
`SSH_AUTH_SOCK`. Git network and `gh` authentication are available only to their dedicated typed
operations; do not move authentication into a build/test command.
