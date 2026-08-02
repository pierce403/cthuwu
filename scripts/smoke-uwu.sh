#!/usr/bin/env bash

set -Eeuo pipefail

readonly SMOKE_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SMOKE_REPO_ROOT="$(cd -- "$SMOKE_SCRIPT_DIR/.." && pwd -P)"

smoke_fail() {
  printf 'uwu.sh smoke test failed: %s\n' "$*" >&2
  exit 1
}

cleanup_smoke_root() {
  if [[ -n "${smoke_writer_pid:-}" ]] && kill -0 "$smoke_writer_pid" 2>/dev/null; then
    kill "$smoke_writer_pid" 2>/dev/null || true
    wait "$smoke_writer_pid" 2>/dev/null || true
  fi
  if [[ -n "${smoke_holder_pid:-}" ]] && kill -0 "$smoke_holder_pid" 2>/dev/null; then
    kill "$smoke_holder_pid" 2>/dev/null || true
    wait "$smoke_holder_pid" 2>/dev/null || true
  fi
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
inbox_fifo="$smoke_root/inbox.fifo"
smoke_holder_pid=""
smoke_writer_pid=""
smoke_environment=(
  env
  NODE_ENV=production
  CARGO_TARGET_DIR="$wrong_target_dir"
  CARGO_BUILD_TARGET=wasm32-unknown-unknown
  RUSTC=/definitely/not/the/validated/rustc
  RUSTC_WRAPPER=/definitely/not/a/rustc-wrapper
)

mkfifo "$inbox_fifo"
(
  cd -- "$smoke_root"
  exec "${smoke_environment[@]}" \
    UWUBOT_DATA_DIR="$state_dir" \
    UWUBOT_XMTP_ENV=dev \
    "$SMOKE_REPO_ROOT/uwu.sh" --stdin-inbox aabbcc <"$inbox_fifo"
) &
smoke_holder_pid=$!
sleep 300 >"$inbox_fifo" &
smoke_writer_pid=$!

for ((attempt = 0; attempt < 1800; attempt += 1)); do
  [[ ! -f "$state_dir/state/environment" ]] || break
  kill -0 "$smoke_holder_pid" 2>/dev/null || break
  sleep 0.1
done

[[ "$(<"$state_dir/state/environment")" == dev ]] || \
  smoke_fail "first start did not persist the dev environment marker"
[[ -d "$state_dir/.uwubot.lock" ]] || \
  smoke_fail "the runtime lock did not remain held across exec"
kill -0 "$smoke_holder_pid" 2>/dev/null || \
  smoke_fail "the first uwubot process exited before the concurrency check"
[[ ! -e "$wrong_target_dir" ]] || \
  smoke_fail "ambient Cargo target configuration escaped the pinned build directory"

set +e
concurrent_output="$(
  (
    cd -- "$smoke_root"
    "${smoke_environment[@]}" \
      "$SMOKE_REPO_ROOT/uwu.sh" \
        --data-dir "$state_dir" \
        --xmtp-env dev \
        --stdin-inbox aabbcc </dev/null 2>&1
  )
)"
concurrent_status=$?
set -e
((concurrent_status != 0)) || \
  smoke_fail "a second launcher used the active data directory"
[[ "$concurrent_output" == *"already running for this data directory"* ]] || \
  smoke_fail "the concurrent-launch rejection was not actionable"

kill "$smoke_writer_pid"
wait "$smoke_writer_pid" 2>/dev/null || true
smoke_writer_pid=""
for ((attempt = 0; attempt < 100; attempt += 1)); do
  kill -0 "$smoke_holder_pid" 2>/dev/null || break
  sleep 0.1
done
kill -0 "$smoke_holder_pid" 2>/dev/null && \
  smoke_fail "the first uwubot process did not stop after stdin closed"
wait "$smoke_holder_pid"
smoke_holder_pid=""
[[ -d "$state_dir/.uwubot.lock" ]] || \
  smoke_fail "the runtime lock did not survive exec for stale-lock recovery"

(
  cd -- "$smoke_root"
  "${smoke_environment[@]}" \
    "$SMOKE_REPO_ROOT/uwu.sh" \
      --data-dir "$state_dir" \
      --xmtp-env dev \
      --stdin-inbox aabbcc </dev/null
)

[[ "$(<"$state_dir/state/environment")" == dev ]] || \
  smoke_fail "second start did not reuse the persisted environment state"
[[ -f "$SMOKE_REPO_ROOT/agent/node_modules/typescript/bin/tsc" ]] || \
  smoke_fail "NODE_ENV=production omitted a required build dependency"

printf 'uwu.sh concurrent and two-start smoke test passed\n'
