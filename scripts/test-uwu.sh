#!/usr/bin/env bash

set -Eeuo pipefail

readonly TEST_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly TEST_REPO_ROOT="$(cd -- "$TEST_SCRIPT_DIR/.." && pwd -P)"
readonly TEST_TEMP_PARENT="$(dirname -- "$TEST_REPO_ROOT")/tmp"

# shellcheck source=../uwu.sh
source "$TEST_REPO_ROOT/uwu.sh"

test_fail() {
  printf 'uwu.sh test failed: %s\n' "$*" >&2
  exit 1
}

test_equal() {
  local expected="$1"
  local actual="$2"
  local label="$3"
  [[ "$actual" == "$expected" ]] || \
    test_fail "$label (expected '$expected', got '$actual')"
}

test_rejected() {
  local label="$1"
  shift
  if ("$@") 2>/dev/null; then
    test_fail "$label was accepted"
  fi
}

cleanup_test_root() {
  if [[ -n "${runtime_holder_pid:-}" ]] && kill -0 "$runtime_holder_pid" 2>/dev/null; then
    kill "$runtime_holder_pid" 2>/dev/null || true
    wait "$runtime_holder_pid" 2>/dev/null || true
  fi
  if [[ -n "${test_root:-}" && -d "$test_root" && \
    "$test_root" == "$TEST_TEMP_PARENT/cthuwu-uwu-test."* ]]; then
    chmod -R u+rwX -- "$test_root"
    rm -rf -- "$test_root"
  fi
}

bash -n "$TEST_REPO_ROOT/uwu.sh"
test_equal "$TEST_REPO_ROOT" "$UWU_REPO_ROOT" "repository resolution"
test_rejected "root launcher execution" uwu_validate_effective_uid 0
uwu_validate_effective_uid 1000

uwu_version_at_least 1.97.0 1.97.0 || test_fail "equal Rust version was rejected"
uwu_version_at_least 1.98.0 1.97.0 || test_fail "newer Rust version was rejected"
test_rejected "older Rust version" uwu_version_at_least 1.96.9 1.97.0
test_rejected "invalid Rust version" uwu_version_at_least banana 1.97.0

for environment in dev production local; do
  uwu_validate_environment "$environment"
done
test_rejected "invalid XMTP environment" uwu_validate_environment staging

mkdir -p -- "$TEST_TEMP_PARENT"
test_root="$(mktemp -d "$TEST_TEMP_PARENT/cthuwu-uwu-test.XXXXXX")"
trap cleanup_test_root EXIT

export UWUBOT_XMTP_ENV=dev
export UWUBOT_DATA_DIR="$test_root/from-environment"
uwu_parse_arguments \
  --xmtp-env production \
  --data-dir "$test_root/state with spaces" \
  --model ollama
test_equal production "$UWU_XMTP_ENV" "CLI environment override"
test_equal "$test_root/state with spaces" "$UWU_DATA_DIR" "CLI data-directory override"
test_equal 1 "$UWU_DATA_DIR_SET" "explicit data-directory tracking"
test_equal 2 "${#UWU_BOT_ARGS[@]}" "forwarded argument count"
test_equal --model "${UWU_BOT_ARGS[0]}" "first forwarded argument"
test_equal ollama "${UWU_BOT_ARGS[1]}" "second forwarded argument"
unset UWUBOT_XMTP_ENV UWUBOT_DATA_DIR
uwu_parse_arguments
test_equal production "$UWU_XMTP_ENV" "default XMTP environment"
export UWUBOT_OPERATOR_ROOT="$test_root/from-workspace-environment"
uwu_parse_arguments --operator-root "$test_root/from-workspace-cli" --model ollama
test_equal "$test_root/from-workspace-cli" "$UWU_OPERATOR_ROOT" "CLI workspace override"
test_equal 2 "${#UWU_BOT_ARGS[@]}" "workspace argument is handled by launcher"
test_rejected "empty CLI workspace" uwu_parse_arguments --operator-root=
unset UWUBOT_OPERATOR_ROOT
uwu_parse_arguments

secret_marker="model-secret-must-not-appear"
search_secret_marker="search-secret-must-not-appear"
set +e
secret_error="$(
  (uwu_parse_arguments "--model-api-key=$secret_marker") 2>&1
)"
secret_status=$?
set -e
((secret_status != 0)) || test_fail "model credential command-line argument was accepted"
[[ "$secret_error" != *"$secret_marker"* ]] || test_fail "model credential appeared in diagnostics"
test_rejected "model credential after option terminator" \
  uwu_parse_arguments -- "--model-api-key=$secret_marker"
test_rejected "split model credential after option terminator" \
  uwu_parse_arguments -- --model-api-key "$secret_marker"
test_rejected "Venice credential argument" \
  uwu_parse_arguments "--venice-api-key=$secret_marker"
test_rejected "Venice credential after option terminator" \
  uwu_parse_arguments -- "--venice-api-key=$secret_marker"
set +e
search_secret_error="$(
  (uwu_parse_arguments "--web-search-api-key=$search_secret_marker") 2>&1
)"
search_secret_status=$?
set -e
((search_secret_status != 0)) || test_fail "web-search credential command-line argument was accepted"
[[ "$search_secret_error" != *"$search_secret_marker"* ]] || \
  test_fail "web-search credential appeared in diagnostics"
test_rejected "web-search credential after option terminator" \
  uwu_parse_arguments -- "--web-search-api-key=$search_secret_marker"
test_rejected "split web-search credential after option terminator" \
  uwu_parse_arguments -- --web-search-api-key "$search_secret_marker"
test_rejected "empty CLI data directory" uwu_parse_arguments --data-dir=

export XMTP_DB_DIRECTORY="$test_root/unsafe-database"
test_rejected "ambient XMTP database override" uwu_validate_ambient_configuration
unset XMTP_DB_DIRECTORY

export XMTP_WALLET_KEY="wallet-key-must-be-file-backed"
test_rejected "ambient XMTP wallet key" uwu_validate_ambient_configuration
unset XMTP_WALLET_KEY

export XMTP_DB_ENCRYPTION_KEY="database-key-must-be-file-backed"
test_rejected "ambient XMTP database key" uwu_validate_ambient_configuration
unset XMTP_DB_ENCRYPTION_KEY

export XDG_DATA_HOME="$test_root/xdg data"
UWU_XMTP_ENV=local
test_equal "$XDG_DATA_HOME/cthuwu/local" \
  "$(uwu_default_data_directory)" "environment-specific default data directory"
unset XDG_DATA_HOME

uwu_ensure_node
prepared="$(uwu_prepare_data_directory "$test_root/state with spaces")"
test_equal "$test_root/state with spaces" "$prepared" "data-directory canonicalization"
mode="$(stat -c '%a' "$prepared" 2>/dev/null || stat -f '%Lp' "$prepared")"
test_equal 700 "$mode" "data-directory permissions"

mkdir -p "$test_root/unrelated-existing"
chmod 755 "$test_root/unrelated-existing"
printf 'unrelated\n' >"$test_root/unrelated-existing/file.txt"
test_rejected "non-Cthuwu existing data directory" \
  uwu_prepare_data_directory "$test_root/unrelated-existing"
mode="$(stat -c '%a' "$test_root/unrelated-existing" 2>/dev/null || \
  stat -f '%Lp' "$test_root/unrelated-existing")"
test_equal 755 "$mode" "rejected directory mode preservation"
[[ -f "$test_root/unrelated-existing/file.txt" ]] || \
  test_fail "rejected directory contents were changed"

mkdir -p "$test_root/real-state"
ln -s "$test_root/real-state" "$test_root/linked-state"
test_rejected "symbolic-link data directory" \
  uwu_prepare_data_directory "$test_root/linked-state"

repo_state="$TEST_REPO_ROOT/.uwu-launcher-test-state"
test_rejected "repository-local data directory" uwu_prepare_data_directory "$repo_state"
[[ ! -e "$repo_state" ]] || test_fail "rejected repository state was created"
test_rejected "repository-containing data directory" \
  uwu_prepare_data_directory "$(dirname -- "$TEST_REPO_ROOT")"

ancestor_state="$TEST_REPO_ROOT/.uwu-ancestor-test-state"
ln -s "$TEST_REPO_ROOT" "$test_root/repository-link"
test_rejected "repository-pointing symlink ancestor" \
  uwu_prepare_data_directory "$test_root/repository-link/.uwu-ancestor-test-state"
[[ ! -e "$ancestor_state" ]] || test_fail "symlink-ancestor state was created in the repository"

UWU_BUILD_LOCK_PATH="$test_root/build.lock"
ln -s "$test_root/real-state" "$UWU_BUILD_LOCK_PATH"
test_rejected "symbolic-link build lock" uwu_acquire_build_lock
rm -f -- "$UWU_BUILD_LOCK_PATH"
mkdir -p "$UWU_BUILD_LOCK_PATH/999999999"
uwu_acquire_build_lock
[[ -d "$UWU_BUILD_LOCK_PATH/$$" ]] || test_fail "stale build lock was not replaced"
uwu_release_build_lock
[[ ! -e "$UWU_BUILD_LOCK_PATH" ]] || \
  test_fail "build lock was not released"

runtime_lock="$test_root/runtime-state/.uwubot.lock"
runtime_ready="$test_root/runtime-lock-ready"
mkdir -p "$(dirname -- "$runtime_lock")"
runtime_holder_pid=""
bash -c '
  set -Eeuo pipefail
  source "$1"
  UWU_RUNTIME_LOCK_PATH="$2"
  uwu_acquire_runtime_lock
  trap uwu_release_runtime_lock EXIT
  : >"$3"
  sleep 2
' bash "$TEST_REPO_ROOT/uwu.sh" "$runtime_lock" "$runtime_ready" &
runtime_holder_pid=$!
for ((attempt = 0; attempt < 50; attempt += 1)); do
  [[ ! -f "$runtime_ready" ]] || break
  sleep 0.1
done
[[ -f "$runtime_ready" ]] || test_fail "runtime-lock holder did not start"
set +e
bash -c '
  set -Eeuo pipefail
  source "$1"
  UWU_RUNTIME_LOCK_PATH="$2"
  uwu_acquire_runtime_lock
' bash "$TEST_REPO_ROOT/uwu.sh" "$runtime_lock" >/dev/null 2>&1
runtime_contender_status=$?
set -e
((runtime_contender_status != 0)) || test_fail "a second runtime acquired the same data lock"
wait "$runtime_holder_pid"
runtime_holder_pid=""
[[ ! -e "$runtime_lock" ]] || test_fail "runtime lock was not released by the test holder"

mkdir -p "$runtime_lock/999999999"
UWU_RUNTIME_LOCK_PATH="$runtime_lock"
uwu_acquire_runtime_lock
[[ -d "$runtime_lock/$$" ]] || test_fail "stale runtime lock was not recovered"
uwu_release_runtime_lock

export UWUBOT_MODEL_API_KEY="$secret_marker"
export UWUBOT_VENICE_API_KEY="$secret_marker"
export VENICE_API_KEY="$secret_marker"
export UWUBOT_WEB_SEARCH_API_KEY="$search_secret_marker"
export XMTP_WALLET_KEY="wallet-secret-must-not-appear"
export XMTP_DB_ENCRYPTION_KEY="database-secret-must-not-appear"
export CARGO_TARGET_DIR="$test_root/untrusted-target-dir"
export CARGO_BUILD_TARGET="untrusted-build-target"
export RUSTC="untrusted-rustc"
export RUSTC_WRAPPER="untrusted-rustc-wrapper"
export RUSTC_WORKSPACE_WRAPPER="untrusted-workspace-wrapper"
uwu_without_runtime_secrets /bin/sh -c '
  test -z "${UWUBOT_MODEL_API_KEY+x}" &&
  test -z "${UWUBOT_VENICE_API_KEY+x}" &&
  test -z "${VENICE_API_KEY+x}" &&
  test -z "${UWUBOT_WEB_SEARCH_API_KEY+x}" &&
  test -z "${XMTP_WALLET_KEY+x}" &&
  test -z "${XMTP_DB_ENCRYPTION_KEY+x}" &&
  test -z "${CARGO_TARGET_DIR+x}" &&
  test -z "${CARGO_BUILD_TARGET+x}" &&
  test -z "${RUSTC+x}" &&
  test -z "${RUSTC_WRAPPER+x}" &&
  test -z "${RUSTC_WORKSPACE_WRAPPER+x}"
' || test_fail "build subprocess inherited runtime secrets"
test_equal "$secret_marker" "$UWUBOT_MODEL_API_KEY" "runtime credential preservation"
test_equal "$secret_marker" "$UWUBOT_VENICE_API_KEY" \
  "runtime namespaced Venice credential preservation"
test_equal "$secret_marker" "$VENICE_API_KEY" \
  "runtime official Venice credential preservation"
test_equal "$search_secret_marker" "$UWUBOT_WEB_SEARCH_API_KEY" \
  "web-search credential preservation"
unset UWUBOT_MODEL_API_KEY UWUBOT_VENICE_API_KEY VENICE_API_KEY
unset UWUBOT_WEB_SEARCH_API_KEY
unset XMTP_WALLET_KEY XMTP_DB_ENCRYPTION_KEY
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET
unset RUSTC RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER

# A selected workspace owns every launcher cache/build destination, including source bootstrap.
UWU_OPERATOR_ROOT="$test_root/operator-workspace"
uwu_prepare_workspace
test_equal "$test_root/operator-workspace/tools/build/uwu" "$UWU_TARGET_DIR" "workspace build target"
test_equal "$test_root/operator-workspace/tools/bootstrap-agent" "$UWU_AGENT_DIR" "workspace sidecar build"
actual_tmp="$(uwu_without_runtime_secrets /bin/sh -c 'printf "%s" "$TMPDIR"')"
test_equal "$UWU_OPERATOR_ROOT/tmp" "$actual_tmp" "workspace child temporary directory"
actual_home="$(uwu_without_runtime_secrets /bin/sh -c 'printf "%s" "$HOME"')"
test_equal "$UWU_OPERATOR_ROOT/tools/home" "$actual_home" "workspace child home"
test_rejected "absent release reported as selected" uwu_select_installed_release

# Existing rustup toolchains and custom Node installations remain readable after HOME/RUSTUP_HOME
# move into an empty workspace. Calling a host rustup proxy here would fail this regression test.
(
  readonly fake_bin="$test_root/custom-node-and-proxies"
  readonly fake_compiler="$test_root/preinstalled-rust-toolchain/bin"
  mkdir -p "$fake_bin" "$fake_compiler"
  cat >"$fake_bin/rustup" <<'RUSTUP'
#!/bin/sh
test "$1" = which || exit 70
printf '%s/%s\n' "$FAKE_COMPILER_BIN" "$2"
RUSTUP
  for name in rustc cargo; do
    printf '#!/bin/sh\nexit 71\n' >"$fake_bin/$name"
    printf '#!/bin/sh\nprintf "preinstalled-%s\\n"\n' "$name" >"$fake_compiler/$name"
  done
  for name in node npm; do
    printf '#!/bin/sh\nprintf "custom-%s\\n"\n' "$name" >"$fake_bin/$name"
  done
  chmod 700 "$fake_bin"/* "$fake_compiler"/*
  export FAKE_COMPILER_BIN="$fake_compiler"
  export PATH="$fake_bin:$PATH"
  UWU_OPERATOR_ROOT="$test_root/toolchain-workspace"
  UWU_WORKSPACE_ENV_READY=0
  uwu_prepare_workspace
  [[ ! -d "$UWU_OPERATOR_ROOT/tools/rustup/toolchains" ]] || test_fail "host toolchain was copied into workspace"
  actual_tools="$(uwu_without_runtime_secrets /bin/sh -c 'rustc --version; cargo --version; node --version; npm --version')"
  test_equal $'preinstalled-rustc\npreinstalled-cargo\ncustom-node\ncustom-npm' "$actual_tools" \
    "child resolves actual host compilers and custom Node with empty workspace Rust home"
  actual_rustup="$(uwu_without_runtime_secrets /bin/sh -c 'printf "%s" "$RUSTUP_HOME"')"
  test_equal "$UWU_OPERATOR_ROOT/tools/rustup" "$actual_rustup" "toolchain access preserves workspace Rust home"
)
python3 "$TEST_REPO_ROOT/scripts/test_release_entrypoint.py"

printf 'uwu.sh tests passed\n'
