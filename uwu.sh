#!/usr/bin/env bash

set -Eeuo pipefail

readonly UWU_MIN_NODE_MAJOR=22
readonly UWU_MIN_RUST_VERSION="1.97.0"
readonly UWU_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly UWU_REPO_ROOT="$UWU_SCRIPT_DIR"
readonly UWU_AGENT_DIR="$UWU_REPO_ROOT/agent"
readonly UWU_MANIFEST="$UWU_REPO_ROOT/cthuwu/Cargo.toml"
readonly UWU_TARGET_DIR="$UWU_REPO_ROOT/cthuwu/target/uwu"

UWU_NODE_BIN=""
UWU_NODE_MAJOR=""
UWU_NODE_PLATFORM=""
UWU_NPM_BIN=""
UWU_CARGO_COMMAND=()
UWU_RUSTC_COMMAND=()
UWU_RUSTC_BIN=""
UWU_RUST_HOST=""
UWU_BINARY=""
UWU_XMTP_ENV=""
UWU_DATA_DIR=""
UWU_DATA_DIR_SET=0
UWU_BOT_ARGS=()
UWU_BUILD_LOCK_PATH="$UWU_REPO_ROOT/cthuwu/target/.uwu-build.lock"
UWU_BUILD_LOCK_HELD=0
UWU_RUNTIME_LOCK_PATH=""
UWU_RUNTIME_LOCK_HELD=0

uwu_log() {
  printf 'uwu.sh: %s\n' "$*" >&2
}

uwu_die() {
  uwu_log "$*"
  exit 1
}

# Toolchain and build helpers must not inherit identity material or model credentials. Persistent
# XMTP keys are file-backed and are removed from the final Rust process too.
uwu_without_runtime_secrets() {
  env \
    -u UWUBOT_MODEL_API_KEY \
    -u UWUBOT_VENICE_API_KEY \
    -u VENICE_API_KEY \
    -u UWUBOT_WEB_SEARCH_API_KEY \
    -u XMTP_WALLET_KEY \
    -u XMTP_DB_ENCRYPTION_KEY \
    -u CARGO_TARGET_DIR \
    -u CARGO_BUILD_TARGET \
    -u RUSTC \
    -u RUSTC_WRAPPER \
    -u RUSTC_WORKSPACE_WRAPPER \
    -u CARGO_BUILD_RUSTC \
    -u CARGO_BUILD_RUSTC_WRAPPER \
    -u CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER \
    "$@"
}

uwu_version_at_least() {
  local available="${1%%[-+]*}"
  local required="${2%%[-+]*}"
  local available_major available_minor available_patch available_extra
  local required_major required_minor required_patch required_extra

  IFS=. read -r available_major available_minor available_patch available_extra <<<"$available"
  IFS=. read -r required_major required_minor required_patch required_extra <<<"$required"
  available_minor="${available_minor:-0}"
  available_patch="${available_patch:-0}"
  required_minor="${required_minor:-0}"
  required_patch="${required_patch:-0}"

  local component
  for component in \
    "$available_major" "$available_minor" "$available_patch" \
    "$required_major" "$required_minor" "$required_patch"; do
    [[ "$component" =~ ^[0-9]+$ ]] || return 1
  done

  if ((10#$available_major != 10#$required_major)); then
    ((10#$available_major > 10#$required_major))
    return
  fi
  if ((10#$available_minor != 10#$required_minor)); then
    ((10#$available_minor > 10#$required_minor))
    return
  fi
  ((10#$available_patch >= 10#$required_patch))
}

uwu_validate_environment() {
  case "$1" in
    dev | production | local) ;;
    *) uwu_die "XMTP environment must be dev, production, or local" ;;
  esac
}

uwu_validate_effective_uid() {
  [[ "$1" != 0 ]] || \
    uwu_die "do not run uwu.sh as root; use a dedicated unprivileged bot account"
}

uwu_reject_unsafe_arguments() {
  local argument
  for argument in "$@"; do
    case "$argument" in
      --model-api-key | --model-api-key=*)
        uwu_die "pass model credentials only through UWUBOT_MODEL_API_KEY, never command-line arguments"
        ;;
      --venice-api-key | --venice-api-key=*)
        uwu_die "pass Venice credentials only through VENICE_API_KEY or UWUBOT_VENICE_API_KEY, never command-line arguments"
        ;;
      --web-search-api-key | --web-search-api-key=*)
        uwu_die "pass web-search credentials only through UWUBOT_WEB_SEARCH_API_KEY, never command-line arguments"
        ;;
      --node | --node=* | --sidecar | --sidecar=*)
        uwu_die "uwu.sh owns the validated Node and sidecar paths; run uwubot directly for transport development"
        ;;
    esac
  done
}

uwu_validate_ambient_configuration() {
  [[ "${XMTP_DB_DIRECTORY+x}" != x ]] || \
    uwu_die "uwu.sh owns XMTP database placement; unset XMTP_DB_DIRECTORY and use UWUBOT_DATA_DIR"
  [[ "${XMTP_WALLET_KEY+x}" != x ]] || \
    uwu_die "XMTP_WALLET_KEY is forbidden; the owner-only persistent identity file owns this key"
  [[ "${XMTP_DB_ENCRYPTION_KEY+x}" != x ]] || \
    uwu_die "XMTP_DB_ENCRYPTION_KEY is forbidden; the owner-only persistent identity file owns this key"
}

uwu_parse_arguments() {
  uwu_reject_unsafe_arguments "$@"
  UWU_XMTP_ENV="${UWUBOT_XMTP_ENV-production}"
  if [[ "${UWUBOT_DATA_DIR+x}" == x ]]; then
    UWU_DATA_DIR="$UWUBOT_DATA_DIR"
    UWU_DATA_DIR_SET=1
  else
    UWU_DATA_DIR=""
    UWU_DATA_DIR_SET=0
  fi
  UWU_BOT_ARGS=()

  while (($# > 0)); do
    case "$1" in
      --xmtp-env)
        (($# >= 2)) || uwu_die "--xmtp-env requires a value"
        UWU_XMTP_ENV="$2"
        shift 2
        ;;
      --xmtp-env=*)
        UWU_XMTP_ENV="${1#*=}"
        shift
        ;;
      --data-dir)
        (($# >= 2)) || uwu_die "--data-dir requires a value"
        UWU_DATA_DIR="$2"
        UWU_DATA_DIR_SET=1
        shift 2
        ;;
      --data-dir=*)
        UWU_DATA_DIR="${1#*=}"
        UWU_DATA_DIR_SET=1
        shift
        ;;
      --model-api-key | --model-api-key=*)
        uwu_die "pass model credentials only through UWUBOT_MODEL_API_KEY, never command-line arguments"
        ;;
      --venice-api-key | --venice-api-key=*)
        uwu_die "pass Venice credentials only through VENICE_API_KEY or UWUBOT_VENICE_API_KEY, never command-line arguments"
        ;;
      --web-search-api-key | --web-search-api-key=*)
        uwu_die "pass web-search credentials only through UWUBOT_WEB_SEARCH_API_KEY, never command-line arguments"
        ;;
      --node | --node=* | --sidecar | --sidecar=*)
        uwu_die "uwu.sh owns the validated Node and sidecar paths; run uwubot directly for transport development"
        ;;
      --)
        UWU_BOT_ARGS+=("$@")
        break
        ;;
      *)
        UWU_BOT_ARGS+=("$1")
        shift
        ;;
    esac
  done

  uwu_validate_environment "$UWU_XMTP_ENV"
  if ((UWU_DATA_DIR_SET == 1)); then
    [[ -n "$UWU_DATA_DIR" ]] || uwu_die "the data directory may not be empty"
  fi
  [[ -z "$UWU_DATA_DIR" || "$UWU_DATA_DIR" != *$'\n'* ]] || \
    uwu_die "the data directory may not contain line breaks"
  [[ -z "$UWU_DATA_DIR" || "$UWU_DATA_DIR" != *$'\r'* ]] || \
    uwu_die "the data directory may not contain line breaks"
}

uwu_ensure_node() {
  command -v node >/dev/null 2>&1 || \
    uwu_die "Node $UWU_MIN_NODE_MAJOR or newer is required; install it and rerun ./uwu.sh"
  command -v npm >/dev/null 2>&1 || \
    uwu_die "npm is required; install it with Node and rerun ./uwu.sh"

  UWU_NODE_BIN="$(command -v node)"
  UWU_NPM_BIN="$(command -v npm)"
  UWU_NODE_MAJOR="$(
    uwu_without_runtime_secrets "$UWU_NODE_BIN" \
      -p 'process.versions.node.split(".")[0]' 2>/dev/null
  )" || uwu_die "could not determine the installed Node version"

  [[ "$UWU_NODE_MAJOR" =~ ^[0-9]+$ ]] || \
    uwu_die "could not determine the installed Node version"
  ((10#$UWU_NODE_MAJOR >= UWU_MIN_NODE_MAJOR)) || \
    uwu_die "Node $UWU_MIN_NODE_MAJOR or newer is required; found Node $UWU_NODE_MAJOR"
  UWU_NODE_PLATFORM="$(
    uwu_without_runtime_secrets "$UWU_NODE_BIN" \
      -p '`${process.platform}-${process.arch}`' 2>/dev/null
  )" || uwu_die "could not determine the Node platform"
  [[ "$UWU_NODE_PLATFORM" =~ ^[a-z0-9._-]+$ ]] || \
    uwu_die "could not determine the Node platform"
}

uwu_configure_rust_commands() {
  local rustc_output cargo_output cargo_verbose rustc_version cargo_version line

  rustc_output="$(
    uwu_without_runtime_secrets "${UWU_RUSTC_COMMAND[@]}" --version 2>/dev/null
  )" || return 1
  cargo_output="$(
    uwu_without_runtime_secrets "${UWU_CARGO_COMMAND[@]}" --version 2>/dev/null
  )" || return 1
  rustc_version="${rustc_output#rustc }"
  rustc_version="${rustc_version%% *}"
  cargo_version="${cargo_output#cargo }"
  cargo_version="${cargo_version%% *}"
  uwu_version_at_least "$rustc_version" "$UWU_MIN_RUST_VERSION" || return 1
  uwu_version_at_least "$cargo_version" "$UWU_MIN_RUST_VERSION" || return 1

  cargo_verbose="$(
    uwu_without_runtime_secrets "${UWU_CARGO_COMMAND[@]}" -vV 2>/dev/null
  )" || return 1
  UWU_RUST_HOST=""
  while IFS= read -r line; do
    case "$line" in
      "host: "*) UWU_RUST_HOST="${line#host: }" ;;
    esac
  done <<<"$cargo_verbose"
  [[ "$UWU_RUST_HOST" =~ ^[A-Za-z0-9._-]+$ ]] || return 1
  UWU_BINARY="$UWU_TARGET_DIR/$UWU_RUST_HOST/release/uwubot"
}

uwu_ensure_rust() {
  local rustc_bin cargo_bin rustup_bin

  if command -v rustc >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    rustc_bin="$(command -v rustc)"
    cargo_bin="$(command -v cargo)"
    UWU_RUSTC_COMMAND=("$rustc_bin")
    UWU_RUSTC_BIN="$rustc_bin"
    UWU_CARGO_COMMAND=("$cargo_bin")
    if uwu_configure_rust_commands; then
      return
    fi
  fi

  command -v rustup >/dev/null 2>&1 || \
    uwu_die "Rust and Cargo $UWU_MIN_RUST_VERSION or newer are required; install them with rustup and rerun ./uwu.sh"
  rustup_bin="$(command -v rustup)"

  if ! uwu_without_runtime_secrets "$rustup_bin" run "$UWU_MIN_RUST_VERSION" \
    rustc --version >/dev/null 2>&1; then
    uwu_log "installing Rust $UWU_MIN_RUST_VERSION with rustup"
    uwu_without_runtime_secrets "$rustup_bin" toolchain install \
      "$UWU_MIN_RUST_VERSION" --profile minimal
  fi

  UWU_RUSTC_COMMAND=("$rustup_bin" run "$UWU_MIN_RUST_VERSION" rustc)
  UWU_CARGO_COMMAND=("$rustup_bin" run "$UWU_MIN_RUST_VERSION" cargo)
  uwu_configure_rust_commands || \
    uwu_die "Rust and Cargo $UWU_MIN_RUST_VERSION are unavailable after rustup installation"
  UWU_RUSTC_BIN="$(
    uwu_without_runtime_secrets "$rustup_bin" which \
      --toolchain "$UWU_MIN_RUST_VERSION" rustc 2>/dev/null
  )" || uwu_die "could not resolve the selected Rust compiler"
  [[ -x "$UWU_RUSTC_BIN" ]] || uwu_die "the selected Rust compiler is not executable"
}

uwu_hash_file() {
  uwu_without_runtime_secrets "$UWU_NODE_BIN" -e '
    const { createHash } = require("node:crypto");
    const { readFileSync } = require("node:fs");
    process.stdout.write(createHash("sha256").update(readFileSync(process.argv[1])).digest("hex"));
  ' "$1"
}

uwu_process_start_identity() {
  local stat_line stat_tail field identity index=0
  if [[ -r "/proc/$1/stat" ]]; then
    IFS= read -r stat_line <"/proc/$1/stat" || return 1
    stat_tail="${stat_line##*) }"
    for field in $stat_tail; do
      ((index += 1))
      if ((index == 20)); then
        [[ "$field" =~ ^[0-9]+$ ]] || return 1
        printf 'proc-start-ticks:%s\n' "$field"
        return
      fi
    done
    return 1
  fi
  identity="$(uwu_without_runtime_secrets ps -o lstart= -p "$1" 2>/dev/null)" || identity=""
  if [[ -n "$identity" ]]; then
    printf 'ps-start:%s\n' "$identity"
    return
  fi
  # Some restricted containers expose neither a usable procfs entry nor process metadata through
  # ps. PID liveness is a conservative fallback: PID reuse can delay startup, but cannot cause the
  # launcher to remove a lock held by a live process it can see.
  if kill -0 "$1" 2>/dev/null; then
    printf 'pid-only:%s\n' "$1"
    return
  fi
  return 1
}

uwu_initialize_lock_owner() {
  local lock_path="$1"
  local identity

  identity="$(uwu_process_start_identity "$$")" || return 1
  [[ -n "$identity" ]] || return 1
  printf '%s\n' "$identity" >"$lock_path/$$/started" || return 1
  chmod 600 -- "$lock_path/$$/started" || return 1
}

uwu_lock_owner_is_active() {
  local owner_path="$1"
  local owner="$2"
  local recorded="" current=""

  [[ -f "$owner_path/started" && ! -L "$owner_path/started" ]] || return 1
  IFS= read -r recorded <"$owner_path/started" || return 1
  [[ -n "$recorded" ]] || return 1
  kill -0 "$owner" 2>/dev/null || return 1
  current="$(uwu_process_start_identity "$owner")" || return 1
  [[ "$current" == "$recorded" ]]
}

uwu_remove_lock_owner() {
  local lock_path="$1"
  local owner="$2"

  rm -f -- "$lock_path/$owner/started" 2>/dev/null || true
  rmdir "$lock_path/$owner" 2>/dev/null || true
  rmdir "$lock_path" 2>/dev/null || true
}

uwu_release_build_lock() {
  if ((UWU_BUILD_LOCK_HELD == 1)); then
    uwu_remove_lock_owner "$UWU_BUILD_LOCK_PATH" "$$"
    UWU_BUILD_LOCK_HELD=0
  fi
}

uwu_acquire_build_lock() {
  local attempt owner="" owner_path

  mkdir -p -- "$(dirname -- "$UWU_BUILD_LOCK_PATH")"
  for ((attempt = 0; attempt < 600; attempt += 1)); do
    [[ ! -L "$UWU_BUILD_LOCK_PATH" ]] || \
      uwu_die "the launcher build-lock path may not be a symbolic link"
    [[ ! -e "$UWU_BUILD_LOCK_PATH" || -d "$UWU_BUILD_LOCK_PATH" ]] || \
      uwu_die "the launcher build-lock path is not a directory"
    if mkdir "$UWU_BUILD_LOCK_PATH" 2>/dev/null; then
      if mkdir "$UWU_BUILD_LOCK_PATH/$$" 2>/dev/null; then
        if uwu_initialize_lock_owner "$UWU_BUILD_LOCK_PATH"; then
          UWU_BUILD_LOCK_HELD=1
          return
        fi
        uwu_remove_lock_owner "$UWU_BUILD_LOCK_PATH" "$$"
        uwu_die "could not record launcher build-lock ownership"
      fi
      rmdir "$UWU_BUILD_LOCK_PATH" 2>/dev/null || true
    fi

    owner=""
    for owner_path in "$UWU_BUILD_LOCK_PATH"/*; do
      if [[ -d "$owner_path" && ! -L "$owner_path" ]]; then
        owner="${owner_path##*/}"
        break
      fi
    done
    if [[ "$owner" =~ ^[0-9]+$ ]] && \
      uwu_lock_owner_is_active "$owner_path" "$owner"; then
      if ((attempt == 0)); then
        uwu_log "another launcher is building this checkout; waiting for it"
      fi
    elif [[ "$owner" =~ ^[0-9]+$ ]]; then
      uwu_remove_lock_owner "$UWU_BUILD_LOCK_PATH" "$owner"
    else
      rmdir "$UWU_BUILD_LOCK_PATH" 2>/dev/null || true
    fi
    uwu_without_runtime_secrets sleep 1
  done

  uwu_die "timed out waiting for another launcher to finish building this checkout"
}

uwu_build_runtime() {
  local lock_file="$UWU_AGENT_DIR/package-lock.json"
  local stamp_file="$UWU_AGENT_DIR/node_modules/.cthuwu-package-lock.sha256"
  local lock_hash expected_stamp existing_stamp=""

  lock_hash="$(uwu_hash_file "$lock_file")"
  expected_stamp="$lock_hash node-$UWU_NODE_MAJOR $UWU_NODE_PLATFORM"
  if [[ -r "$stamp_file" ]]; then
    IFS= read -r existing_stamp <"$stamp_file" || true
  fi

  rm -f -- "$stamp_file"
  if [[ "$existing_stamp" != "$expected_stamp" || \
    ! -f "$UWU_AGENT_DIR/node_modules/@xmtp/agent-sdk/package.json" || \
    ! -f "$UWU_AGENT_DIR/node_modules/@xmtp/node-bindings/package.json" || \
    ! -f "$UWU_AGENT_DIR/node_modules/typescript/bin/tsc" || \
    ! -f "$UWU_AGENT_DIR/node_modules/vitest/package.json" ]]; then
    uwu_log "installing locked XMTP sidecar dependencies"
    uwu_without_runtime_secrets "$UWU_NPM_BIN" --prefix "$UWU_AGENT_DIR" \
      ci --include=dev --no-audit --no-fund
  fi

  uwu_log "building the XMTP sidecar"
  uwu_without_runtime_secrets "$UWU_NPM_BIN" --prefix "$UWU_AGENT_DIR" run build
  printf '%s\n' "$expected_stamp" >"$stamp_file"
  chmod 600 -- "$stamp_file"

  uwu_log "building uwubot"
  uwu_without_runtime_secrets env RUSTC="$UWU_RUSTC_BIN" \
    "${UWU_CARGO_COMMAND[@]}" build \
    --manifest-path "$UWU_MANIFEST" \
    --package cthuwu \
    --target-dir "$UWU_TARGET_DIR" \
    --target "$UWU_RUST_HOST" \
    --release \
    --locked
  [[ -x "$UWU_BINARY" ]] || uwu_die "Cargo did not produce the expected uwubot executable"
}

uwu_default_data_directory() {
  local base
  if [[ -n "${XDG_DATA_HOME:-}" ]]; then
    base="$XDG_DATA_HOME"
    [[ "$base" == /* ]] || uwu_die "XDG_DATA_HOME must be an absolute path"
  elif [[ -n "${HOME:-}" ]]; then
    base="$HOME/.local/share"
    [[ "$base" == /* ]] || uwu_die "HOME must be an absolute path"
  else
    uwu_die "set UWUBOT_DATA_DIR, XDG_DATA_HOME, or HOME to choose persistent storage"
  fi
  printf '%s/cthuwu/%s\n' "${base%/}" "$UWU_XMTP_ENV"
}

uwu_resolve_path() {
  uwu_without_runtime_secrets "$UWU_NODE_BIN" -e '
    const { resolve } = require("node:path");
    process.stdout.write(resolve(process.argv[1]));
  ' "$1"
}

uwu_path_is_in_repository() {
  case "$1/" in
    "$UWU_REPO_ROOT/"*) return 0 ;;
    *) return 1 ;;
  esac
}

uwu_path_overlaps_repository() {
  local candidate="${1%/}"
  [[ -n "$candidate" ]] || candidate="/"
  [[ "$candidate" != / ]] || return 0
  uwu_path_is_in_repository "$candidate" && return 0
  case "$UWU_REPO_ROOT/" in
    "$candidate/"*) return 0 ;;
    *) return 1 ;;
  esac
}

uwu_existing_data_directory_is_dedicated() (
  local directory="$1"
  local marker="$directory/state/environment"
  local entries=()

  if [[ -f "$marker" && ! -L "$directory/state" && ! -L "$marker" ]]; then
    [[ "$(<"$marker")" == "$UWU_XMTP_ENV" ]]
    return
  fi

  shopt -s dotglob nullglob
  entries=("$directory"/*)
  if ((${#entries[@]} == 0)); then
    return 0
  fi
  if ((${#entries[@]} == 1)) && \
    [[ "${entries[0]}" == "$directory/.uwubot.lock" && \
      -d "${entries[0]}" && ! -L "${entries[0]}" ]]; then
    return 0
  fi
  return 1
)

uwu_prepare_data_directory() {
  local requested="$1"
  local resolved canonical canonical_candidate existing parent suffix

  [[ -n "$requested" ]] || uwu_die "the data directory may not be empty"
  resolved="$(uwu_resolve_path "$requested")"
  uwu_path_overlaps_repository "$resolved" && \
    uwu_die "persistent bot state must not be inside or contain the repository"
  [[ ! -L "$resolved" ]] || uwu_die "the data directory may not be a symbolic link"

  existing="$resolved"
  while [[ ! -e "$existing" && ! -L "$existing" ]]; do
    parent="${existing%/*}"
    [[ -n "$parent" ]] || parent="/"
    [[ "$parent" != "$existing" ]] || break
    existing="$parent"
  done
  [[ -d "$existing" ]] || uwu_die "an existing data-directory parent is not a directory"
  canonical="$(cd -- "$existing" && pwd -P)" || \
    uwu_die "could not resolve the existing data-directory parent"
  suffix="${resolved#"$existing"}"
  canonical_candidate="$(uwu_resolve_path "$canonical$suffix")"
  uwu_path_overlaps_repository "$canonical_candidate" && \
    uwu_die "persistent bot state must not be inside or contain the repository"

  if [[ -d "$resolved" ]]; then
    uwu_existing_data_directory_is_dedicated "$resolved" || \
      uwu_die "an existing data directory must be empty or contain matching Cthuwu state"
  fi

  mkdir -p -- "$resolved" || uwu_die "could not create the data directory"
  [[ -d "$resolved" && ! -L "$resolved" ]] || \
    uwu_die "the data directory must be a real directory"

  canonical="$(cd -- "$resolved" && pwd -P)" || \
    uwu_die "could not resolve the data directory"
  uwu_path_overlaps_repository "$canonical" && \
    uwu_die "persistent bot state must not be inside or contain the repository"
  [[ -O "$resolved" ]] || uwu_die "the data directory must be owned by the current user"
  chmod 700 -- "$resolved" || uwu_die "could not restrict data-directory permissions"
  printf '%s\n' "$canonical"
}

uwu_release_runtime_lock() {
  if ((UWU_RUNTIME_LOCK_HELD == 1)); then
    uwu_remove_lock_owner "$UWU_RUNTIME_LOCK_PATH" "$$"
    UWU_RUNTIME_LOCK_HELD=0
  fi
}

uwu_acquire_runtime_lock() {
  local attempt owner="" owner_path

  for ((attempt = 0; attempt < 4; attempt += 1)); do
    [[ ! -L "$UWU_RUNTIME_LOCK_PATH" ]] || \
      uwu_die "the runtime lock may not be a symbolic link"
    [[ ! -e "$UWU_RUNTIME_LOCK_PATH" || -d "$UWU_RUNTIME_LOCK_PATH" ]] || \
      uwu_die "the runtime lock path is not a directory"

    if mkdir "$UWU_RUNTIME_LOCK_PATH" 2>/dev/null; then
      if mkdir "$UWU_RUNTIME_LOCK_PATH/$$" 2>/dev/null; then
        if uwu_initialize_lock_owner "$UWU_RUNTIME_LOCK_PATH"; then
          UWU_RUNTIME_LOCK_HELD=1
          return
        fi
        uwu_remove_lock_owner "$UWU_RUNTIME_LOCK_PATH" "$$"
        uwu_die "could not record runtime-lock ownership"
      fi
      rmdir "$UWU_RUNTIME_LOCK_PATH" 2>/dev/null || true
    fi

    owner=""
    for owner_path in "$UWU_RUNTIME_LOCK_PATH"/*; do
      if [[ -d "$owner_path" && ! -L "$owner_path" ]]; then
        owner="${owner_path##*/}"
        break
      fi
    done
    if [[ "$owner" =~ ^[0-9]+$ ]] && \
      uwu_lock_owner_is_active "$owner_path" "$owner"; then
      uwu_die "uwubot is already running for this data directory"
    elif [[ "$owner" =~ ^[0-9]+$ ]]; then
      uwu_remove_lock_owner "$UWU_RUNTIME_LOCK_PATH" "$owner"
    else
      rmdir "$UWU_RUNTIME_LOCK_PATH" 2>/dev/null || true
    fi
  done

  uwu_die "could not acquire the runtime lock for this data directory"
}

uwu_release_all_locks() {
  uwu_release_build_lock
  uwu_release_runtime_lock
}

uwu_main() {
  umask 077
  uwu_validate_effective_uid "$EUID"
  uwu_parse_arguments "$@"
  uwu_validate_ambient_configuration
  uwu_ensure_node

  if ((UWU_DATA_DIR_SET == 0)); then
    UWU_DATA_DIR="$(uwu_default_data_directory)"
  fi
  UWU_DATA_DIR="$(uwu_prepare_data_directory "$UWU_DATA_DIR")"

  export UWUBOT_DATA_DIR="$UWU_DATA_DIR"
  export UWUBOT_XMTP_ENV="$UWU_XMTP_ENV"
  export UWUBOT_NODE="$UWU_NODE_BIN"
  export UWUBOT_SIDECAR="$UWU_AGENT_DIR/dist/index.js"
  export UWUBOT_OPERATOR_ROOT="${UWUBOT_OPERATOR_ROOT:-$UWU_REPO_ROOT}"

  UWU_RUNTIME_LOCK_PATH="$UWU_DATA_DIR/.uwubot.lock"
  uwu_acquire_runtime_lock
  trap uwu_release_all_locks EXIT

  uwu_ensure_rust
  uwu_acquire_build_lock
  uwu_build_runtime
  uwu_release_build_lock

  uwu_log "starting uwubot on XMTP $UWU_XMTP_ENV"
  uwu_log "persistent state: $UWU_DATA_DIR"
  uwu_log "console activity shows XMTP delivery, thinking, and tool phases; message bodies and secrets stay private"
  exec env -u XMTP_WALLET_KEY -u XMTP_DB_ENCRYPTION_KEY \
    "$UWU_BINARY" "${UWU_BOT_ARGS[@]}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  uwu_main "$@"
fi
