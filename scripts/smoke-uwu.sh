#!/usr/bin/env bash

set -Eeuo pipefail

readonly SMOKE_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SMOKE_REPO_ROOT="$(cd -- "$SMOKE_SCRIPT_DIR/.." && pwd -P)"

smoke_fail() {
  printf 'uwu.sh smoke test failed: %s\n' "$*" >&2
  exit 1
}

cleanup_smoke_root() {
  if [[ -n "${smoke_root:-}" && -d "$smoke_root" && \
    "$smoke_root" == "${TMPDIR:-/tmp}/cthuwu-uwu-smoke."* ]]; then
    chmod -R u+rwX -- "$smoke_root"
    rm -rf -- "$smoke_root"
  fi
}

smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/cthuwu-uwu-smoke.XXXXXX")"
trap cleanup_smoke_root EXIT
state_dir="$smoke_root/persistent state"
wrong_target_dir="$smoke_root/ambient cargo target"
smoke_environment=(
  env
  NODE_ENV=production
  CARGO_TARGET_DIR="$wrong_target_dir"
  CARGO_BUILD_TARGET=wasm32-unknown-unknown
  RUSTC=/definitely/not/the/validated/rustc
  RUSTC_WRAPPER=/definitely/not/a/rustc-wrapper
)

(
  cd -- "$smoke_root"
  "${smoke_environment[@]}" \
    UWUBOT_DATA_DIR="$state_dir" \
    "$SMOKE_REPO_ROOT/uwu.sh" operator list
)

[[ "$(<"$state_dir/state/environment")" == production ]] || \
  smoke_fail "first start did not persist the default production environment marker"
[[ ! -e "$wrong_target_dir" ]] || \
  smoke_fail "ambient Cargo target configuration escaped the pinned build directory"

(
  cd -- "$smoke_root"
  "${smoke_environment[@]}" \
    "$SMOKE_REPO_ROOT/uwu.sh" \
      --data-dir "$state_dir" \
      operator list
)

[[ "$(<"$state_dir/state/environment")" == production ]] || \
  smoke_fail "second start did not reuse the persisted environment state"
[[ -f "$SMOKE_REPO_ROOT/agent/node_modules/typescript/bin/tsc" ]] || \
  smoke_fail "NODE_ENV=production omitted a required build dependency"

printf 'uwu.sh production-default two-start smoke test passed\n'
