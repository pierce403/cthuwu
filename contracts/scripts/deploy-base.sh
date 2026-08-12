#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  deploy-base.sh --deployer 0x... [wallet source] [options]

Wallet source (choose exactly one unless --status-only is used):
  --account NAME          Foundry encrypted keystore account
  --keystore PATH        Foundry encrypted keystore file
  --ledger               Ledger hardware wallet
  --trezor               Trezor hardware wallet

Options:
  --password-file PATH   Owner-only password file outside the worktree
  --state-dir PATH       Durable external state root
  --confirmations N      Required canonical Base confirmations (default: 12)
  --poll-seconds N       Underfunding recheck interval (default: 60)
  --status-only          Dry-run/estimate/emit any due funding block, then exit
  --notify-now           Treat this invocation as an explicit funding status request
  --help                 Show this text

Required environment:
  BASE_RPC_URL           Base-mainnet JSON-RPC endpoint (never written to state)

Raw private keys, mnemonics, --private-key, PRIVATE_KEY-style environment variables,
unlocked RPC accounts, and generic remote signers are deliberately unsupported.
USAGE
}

die() {
  printf 'deploy-base: %s\n' "$*" >&2
  exit 1
}

require_value() {
  local option_name="$1"
  local option_value="${2:-}"
  [[ -n "${option_value}" && "${option_value}" != --* ]] || die "${option_name} requires a value"
}

deployer=""
state_dir_input=""
confirmations=12
poll_seconds=60
status_only=false
notify_now=false
wallet_source=""
wallet_value=""
password_file=""

while (( $# > 0 )); do
  case "$1" in
    --deployer)
      require_value "$1" "${2:-}"
      deployer="$2"
      shift 2
      ;;
    --state-dir)
      require_value "$1" "${2:-}"
      state_dir_input="$2"
      shift 2
      ;;
    --confirmations)
      require_value "$1" "${2:-}"
      confirmations="$2"
      shift 2
      ;;
    --poll-seconds)
      require_value "$1" "${2:-}"
      poll_seconds="$2"
      shift 2
      ;;
    --account|--keystore)
      require_value "$1" "${2:-}"
      [[ -z "${wallet_source}" ]] || die "choose exactly one wallet source"
      wallet_source="$1"
      wallet_value="$2"
      shift 2
      ;;
    --ledger|--trezor)
      [[ -z "${wallet_source}" ]] || die "choose exactly one wallet source"
      wallet_source="$1"
      shift
      ;;
    --password-file)
      require_value "$1" "${2:-}"
      password_file="$2"
      shift 2
      ;;
    --status-only)
      status_only=true
      shift
      ;;
    --notify-now)
      notify_now=true
      shift
      ;;
    --private-key|--private-keys|--mnemonic|--mnemonics|--interactive|--unlocked|--aws|--gcp)
      die "$1 is prohibited for this deployment workflow"
      ;;
    --private-key=*|--private-keys=*|--mnemonic=*|--mnemonics=*|--password=*|--aws=*|--gcp=*)
      die "raw secret or remote-signer arguments are prohibited for this deployment workflow"
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      if [[ "$1" == --* ]]; then
        unknown_option_name="${1%%=*}"
        die "unknown option: ${unknown_option_name}"
      fi
      die "unexpected positional argument"
      ;;
  esac
done

[[ "${deployer}" =~ ^0x[0-9a-fA-F]{40}$ ]] || die "--deployer must be a full EVM address"
[[ ! "${deployer}" =~ ^0x0{40}$ ]] || die "--deployer must not be zero"
deployer="${deployer,,}"
[[ "${confirmations}" =~ ^[1-9][0-9]{0,3}$ ]] || die "--confirmations must be an integer from 1 to 10000"
(( confirmations <= 10000 )) || die "--confirmations must be an integer from 1 to 10000"
[[ "${poll_seconds}" =~ ^[1-9][0-9]{0,3}$ ]] || die "--poll-seconds must be an integer from 5 to 3600"
(( poll_seconds >= 5 && poll_seconds <= 3600 )) || die "--poll-seconds must be an integer from 5 to 3600"

if [[ "${status_only}" == false ]]; then
  [[ -n "${wallet_source}" ]] || die "a Foundry encrypted keystore or hardware wallet is required"
fi
if [[ -n "${password_file}" ]]; then
  [[ "${wallet_source}" == "--account" || "${wallet_source}" == "--keystore" ]] \
    || die "--password-file is valid only with --account or --keystore"
  [[ -f "${password_file}" && ! -L "${password_file}" ]] || die "--password-file must name a regular non-symlink file"
  password_mode="$(stat -c '%a' -- "${password_file}")"
  (( (8#${password_mode} & 8#077) == 0 )) || die "--password-file must not be accessible by group or others"
fi
if [[ "${wallet_source}" == "--keystore" ]]; then
  [[ -f "${wallet_value}" && ! -L "${wallet_value}" ]] || die "--keystore must name a regular non-symlink file"
fi

while IFS='=' read -r environment_name _; do
  if [[ "${environment_name}" =~ (^|_)PRIVATE_KEYS?($|_)|(^|_)MNEMONICS?($|_)|^ETH_PASSWORD$ ]]; then
    die "refusing secret-bearing environment variable ${environment_name}; use an encrypted keystore or hardware wallet"
  fi
done < <(env)

base_rpc_url="${BASE_RPC_URL:-}"
[[ -n "${base_rpc_url}" ]] || die "BASE_RPC_URL is required"
[[ "${base_rpc_url}" == http://* || "${base_rpc_url}" == https://* ]] || die "BASE_RPC_URL must be HTTP(S)"

command -v forge >/dev/null 2>&1 || die "Foundry forge is required"
command -v node >/dev/null 2>&1 || die "Node.js is required"
command -v flock >/dev/null 2>&1 || die "flock is required for single-writer deployment state"
command -v git >/dev/null 2>&1 || die "Git is required to verify exact dependency provenance"
forge_version="$(forge --version 2>/dev/null)"
forge_version_line="${forge_version%%$'\n'*}"
[[ "${forge_version_line}" =~ ^forge[[:space:]]Version:[[:space:]]1\.7\.1([-+][^[:space:]]+)?$ ]] \
  || die "Foundry v1.7.1 release is required exactly"
node_major="$(node -p 'process.versions.node.split(".")[0]')"
(( node_major >= 22 )) || die "Node.js 22 or newer is required for package-free TypeScript"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
contracts_root="$(cd -- "${script_dir}/.." && pwd -P)"
repo_root="$(cd -- "${contracts_root}/.." && pwd -P)"

secure_private_file=""
validate_private_file() {
  local label="$1"
  local input_path="$2"
  local input_dir
  local canonical_path
  local file_mode
  [[ -f "${input_path}" && ! -L "${input_path}" ]] || die "${label} must name a regular non-symlink file"
  input_dir="$(cd -- "$(dirname -- "${input_path}")" && pwd -P)"
  canonical_path="${input_dir}/$(basename -- "${input_path}")"
  [[ "$(stat -c '%u' -- "${canonical_path}")" == "$(id -u)" ]] || die "${label} must be owned by the current user"
  file_mode="$(stat -c '%a' -- "${canonical_path}")"
  (( (8#${file_mode} & 8#077) == 0 )) || die "${label} must not be accessible by group or others"
  case "${canonical_path}" in
    "${repo_root}"|"${repo_root}/"*) die "${label} must live outside the git worktree" ;;
  esac
  secure_private_file="${canonical_path}"
}

if [[ -n "${password_file}" ]]; then
  validate_private_file "--password-file" "${password_file}"
  password_file="${secure_private_file}"
fi
if [[ "${wallet_source}" == "--keystore" ]]; then
  validate_private_file "--keystore" "${wallet_value}"
  wallet_value="${secure_private_file}"
fi

if [[ -z "${state_dir_input}" ]]; then
  state_home="${XDG_STATE_HOME:-${HOME:?HOME is required when XDG_STATE_HOME is unset}/.local/state}"
  state_dir_input="${state_home}/cthuwu/acolyte-branding"
fi
[[ ! -L "${state_dir_input}" ]] || die "--state-dir must not be a symbolic link"
mkdir -p -- "${state_dir_input}"
state_dir="$(cd -- "${state_dir_input}" && pwd -P)"
case "${state_dir}/" in
  "${repo_root}/"*) die "deployment state must live outside the git worktree" ;;
esac
[[ "$(stat -c '%u' -- "${state_dir}")" == "$(id -u)" ]] || die "deployment state directory must be owned by the current user"
chmod 700 -- "${state_dir}"
umask 077

workflow_path="${state_dir}/workflow.json"
deployment_path="${state_dir}/base-mainnet.json"
broadcast_root="${state_dir}/broadcast"
broadcast_path="${broadcast_root}/DeployAcolyteBranding.s.sol/8453/run-latest.json"
artifact_path="${contracts_root}/out/CthuwuAcolyteBranding.sol/CthuwuAcolyteBranding.json"
tool_path="${script_dir}/estimate-deployment-funding.ts"
deploy_script="script/DeployAcolyteBranding.s.sol:DeployAcolyteBranding"
verify_script="script/VerifyAcolyteBranding.s.sol:VerifyAcolyteBranding"
node --experimental-strip-types --check "${tool_path}" >/dev/null 2>&1 \
  || die "Node.js must support package-free TypeScript stripping"

exec 9>"${state_dir}/workflow.lock"
flock -n 9 || die "another Acolyte Branding deployment workflow is already running"
chmod 600 -- "${state_dir}/workflow.lock"

export CTHUWU_BRANDING_RPC_URL="${base_rpc_url}"
export FOUNDRY_ETH_RPC_URL="${base_rpc_url}"
export FOUNDRY_BROADCAST="${broadcast_root}"
export FOUNDRY_CACHE_PATH="${state_dir}/foundry-cache"
export FOUNDRY_PROFILE=default
unset base_rpc_url BASE_RPC_URL

node_tool() {
  node --experimental-strip-types "${tool_path}" "$@"
}

state_field() {
  node_tool read --state "${workflow_path}" --field "$1"
}

run_preflight() {
  (
    cd -- "${contracts_root}"
    forge script "${deploy_script}" \
      --sig 'preflight(address)' "${deployer}" \
      --slow
  )
}

wallet_args=()
case "${wallet_source}" in
  --account|--keystore)
    wallet_args+=("${wallet_source}" "${wallet_value}")
    ;;
  --ledger|--trezor)
    wallet_args+=("${wallet_source}")
    ;;
esac
if [[ -n "${password_file}" ]]; then
  wallet_args+=(--password-file "${password_file}")
fi
run_broadcast() {
  (
    cd -- "${contracts_root}"
    forge script "${deploy_script}" \
      --sig 'run(address)' "${deployer}" \
      --sender "${deployer}" \
      --slow \
      --broadcast \
      "${wallet_args[@]}"
  )
}

run_resume() {
  (
    cd -- "${contracts_root}"
    forge script "${deploy_script}" \
      --sig 'run(address)' "${deployer}" \
      --sender "${deployer}" \
      --slow \
      --resume \
      "${wallet_args[@]}"
  )
}

verify_deployment() {
  local branding_address="$1"
  (
    cd -- "${contracts_root}"
    forge script "${verify_script}" \
      --sig 'run(address)' "${branding_address}" \
      --sender "${deployer}"
  )
}

ensure_artifact() {
  # Rebuild even when an artifact exists so source drift cannot survive into signing,
  # resume, or canonical finalization. The Node gate then compares the rebuilt hash
  # with the durable estimate and pre-broadcast intent.
  (
    cd -- "${contracts_root}"
    forge build
  )
  [[ -f "${artifact_path}" ]] || die "Foundry did not produce the expected Branding artifact"
}

inspect_saved_broadcast() {
  [[ -f "${broadcast_path}" ]] || return 1
  ensure_artifact
  node_tool inspect-broadcast \
    --artifact "${artifact_path}" \
    --broadcast "${broadcast_path}" \
    --state "${workflow_path}"
}

finalize_submitted() {
  local receipt_status
  receipt_status="$(node_tool reconcile --state "${workflow_path}")"
  if [[ "${receipt_status}" == "pending" ]]; then
    printf 'A submitted deployment has no receipt yet; resuming the exact Foundry broadcast state.\n' >&2
    [[ -f "${broadcast_path}" ]] || die "submitted transaction is pending but its exact Foundry resume artifact is missing"
    inspect_saved_broadcast >/dev/null
    run_resume
    inspect_saved_broadcast >/dev/null
  fi
  ensure_artifact
  node_tool finalize \
    --artifact "${artifact_path}" \
    --state "${workflow_path}" \
    --deployment "${deployment_path}" \
    --confirmations "${confirmations}" \
    --timeout-seconds 1800 >/dev/null
  local branding_address
  branding_address="$(state_field contractAddress)"
  verify_deployment "${branding_address}"
  printf 'Canonical Base deployment provenance: %s\n' "${deployment_path}"
}

report_submitted_status() {
  local receipt_status
  receipt_status="$(node_tool reconcile --state "${workflow_path}")"
  printf 'ACOLYTE BRANDING DEPLOYMENT SUBMITTED\nDeployer: %s\nContract: %s\nTransaction hash: %s\nReceipt status: %s\nChain: Base mainnet\nChain ID: 8453\n' \
    "${deployer}" \
    "$(state_field contractAddress)" \
    "$(state_field transactionHash)" \
    "${receipt_status}"
}

revalidate_confirmed() {
  ensure_artifact
  node_tool finalize \
    --artifact "${artifact_path}" \
    --state "${workflow_path}" \
    --deployment "${deployment_path}" \
    --confirmations "${confirmations}" \
    --timeout-seconds 1800 >/dev/null
  verify_deployment "$(state_field contractAddress)"
  printf 'Existing canonical Base deployment strictly revalidated: %s\n' "${deployment_path}"
}

broadcast_prepared() {
  ensure_artifact
  node_tool prepare-broadcast --artifact "${artifact_path}" --state "${workflow_path}" >/dev/null
  set +e
  run_broadcast
  broadcast_status=$?
  set -e
  if [[ -f "${broadcast_path}" ]]; then
    inspect_saved_broadcast >/dev/null
  fi
  if (( broadcast_status != 0 )); then
    if [[ "$(state_field phase)" == "submitted" ]]; then
      die "Foundry broadcast did not complete; durable state was retained and the next run will resume it"
    fi
    die "Foundry broadcast failed; the durable pre-broadcast intent was retained for nonce reconciliation"
  fi
  [[ "$(state_field phase)" == "submitted" ]] || die "successful Foundry broadcast produced no durable submission"
  finalize_submitted
}

if [[ -f "${deployment_path}" ]]; then
  [[ -f "${workflow_path}" ]] || die "canonical deployment exists without its durable workflow state"
  existing_phase="$(state_field phase)"
  if [[ "${existing_phase}" == "confirmed" ]]; then
    revalidate_confirmed
    exit 0
  fi
  [[ "${existing_phase}" == "submitted" ]] \
    || die "canonical deployment exists but workflow state is neither submitted nor confirmed"
  if [[ "${status_only}" == true ]]; then
    report_submitted_status
    exit 0
  fi
  finalize_submitted
  exit 0
fi

if [[ -f "${broadcast_path}" ]]; then
  inspect_saved_broadcast >/dev/null
fi
if [[ -f "${workflow_path}" && "$(state_field phase)" == "submitted" ]]; then
  if [[ "${status_only}" == true ]]; then
    report_submitted_status
    exit 0
  fi
  finalize_submitted
  exit 0
fi
if [[ -f "${workflow_path}" && "$(state_field phase)" == "prepared" ]]; then
  ensure_artifact
  node_tool prepare-broadcast --artifact "${artifact_path}" --state "${workflow_path}" >/dev/null
  if [[ "${status_only}" == true ]]; then
    printf 'ACOLYTE BRANDING DEPLOYMENT PREPARED\nDeployer: %s\nPredicted contract: %s\nReceipt status: not-submitted\nChain: Base mainnet\nChain ID: 8453\n' \
      "${deployer}" \
      "$(state_field predictedContractAddress)"
    exit 0
  fi
  broadcast_prepared
  exit 0
fi

dry_run_current=false
if [[ ! -f "${workflow_path}" ]]; then
  # This non-recording Solidity preflight executes the exact constructor and runtime sanity
  # checks without requiring the not-yet-funded sender to pay simulated gas. The immediately
  # following TypeScript estimate executes the exact direct-CREATE input with the real sender
  # and pending nonce, then adds Base's L1 fee and reads the sender's real pending balance.
  run_preflight
  dry_run_current=true
fi

while true; do
  [[ -f "${artifact_path}" ]] || die "Foundry did not produce the expected Branding artifact"
  node_tool estimate \
    --artifact "${artifact_path}" \
    --deployer "${deployer}" \
    --state "${workflow_path}" >/dev/null
  phase="$(state_field phase)"

  if [[ "${phase}" == "funding_required" ]]; then
    notification_args=(notification-status --state "${workflow_path}")
    if [[ "${notify_now}" == true || "${status_only}" == true ]]; then
      notification_args+=(--explicit)
    fi
    notification_reason="$(node_tool "${notification_args[@]}")"
    if [[ "${notification_reason}" != "not-due" ]]; then
      # This exact stdout block traverses XMTP only when the wrapper is invoked by the existing
      # authenticated operator exact-exec path. There is intentionally no standalone or generic
      # XMTP sender here; a local invocation emits only to its terminal.
      node_tool notification-message --state "${workflow_path}"
      node_tool record-notification --state "${workflow_path}"
    fi
    if [[ "${status_only}" == true ]]; then
      exit 75
    fi
    printf 'Waiting for adequate Base funding; next balance check in %s seconds.\n' "${poll_seconds}" >&2
    sleep "${poll_seconds}"
    dry_run_current=false
    notify_now=false
    continue
  fi

  [[ "${phase}" == "ready" ]] || die "unexpected deployment workflow phase: ${phase}"
  if [[ "${status_only}" == true ]]; then
    printf 'ACOLYTE BRANDING DEPLOYMENT FUNDING IS ADEQUATE\nDeployer: %s\nChain: Base mainnet\nChain ID: 8453\n' "${deployer}"
    exit 0
  fi

  if [[ "${dry_run_current}" == false ]]; then
    run_preflight
    node_tool estimate \
      --artifact "${artifact_path}" \
      --deployer "${deployer}" \
      --state "${workflow_path}" >/dev/null
    [[ "$(state_field phase)" == "ready" ]] || {
      dry_run_current=true
      continue
    }
  fi

  broadcast_prepared
  exit 0
done
