#!/usr/bin/env bash

set -Eeuo pipefail

readonly CTHUWU_INSTALL_REPOSITORY="https://github.com/pierce403/cthuwu.git"
readonly CTHUWU_INSTALL_BRANCH="main"

CTHUWU_INSTALL_OPERATOR=""
CTHUWU_INSTALL_DIRECTORY=""
CTHUWU_INSTALL_DATA_DIRECTORY=""
CTHUWU_INSTALL_LAUNCH_ARGS=()

cthuwu_install_log() {
  printf 'cthuwu install: %s\n' "$*" >&2
}

cthuwu_install_die() {
  cthuwu_install_log "$*"
  exit 1
}

cthuwu_install_default_directory() {
  local data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
  printf '%s/cthuwu/source\n' "$data_home"
}

cthuwu_install_default_data_directory() {
  local data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
  printf '%s/cthuwu/tentacle\n' "$data_home"
}

cthuwu_install_validate_operator() {
  [[ "$1" =~ ^0x[0-9A-Fa-f]{40}$ ]] || \
    cthuwu_install_die "--operator must be the Acolyte's full public Ethereum address"
  [[ "$1" != "0x0000000000000000000000000000000000000000" ]] || \
    cthuwu_install_die "--operator may not be the zero address"
}

cthuwu_install_parse_arguments() {
  CTHUWU_INSTALL_OPERATOR=""
  CTHUWU_INSTALL_DIRECTORY="${CTHUWU_INSTALL_DIR:-$(cthuwu_install_default_directory)}"
  CTHUWU_INSTALL_DATA_DIRECTORY="${CTHUWU_INSTALL_DATA_DIR:-$(cthuwu_install_default_data_directory)}"
  while (($# > 0)); do
    case "$1" in
      --operator)
        (($# >= 2)) || cthuwu_install_die "--operator requires an address"
        CTHUWU_INSTALL_OPERATOR="$2"
        shift 2
        ;;
      --operator=*)
        CTHUWU_INSTALL_OPERATOR="${1#*=}"
        shift
        ;;
      --install-dir)
        (($# >= 2)) || cthuwu_install_die "--install-dir requires a path"
        CTHUWU_INSTALL_DIRECTORY="$2"
        shift 2
        ;;
      --install-dir=*)
        CTHUWU_INSTALL_DIRECTORY="${1#*=}"
        shift
        ;;
      --data-dir)
        (($# >= 2)) || cthuwu_install_die "--data-dir requires a path"
        CTHUWU_INSTALL_DATA_DIRECTORY="$2"
        shift 2
        ;;
      --data-dir=*)
        CTHUWU_INSTALL_DATA_DIRECTORY="${1#*=}"
        shift
        ;;
      --help)
        printf 'usage: install.sh --operator 0x... [--install-dir /path/to/source] [--data-dir /fresh/tentacle/state]\n'
        exit 0
        ;;
      *) cthuwu_install_die "unknown installer argument: $1" ;;
    esac
  done
  [[ -n "$CTHUWU_INSTALL_OPERATOR" ]] || \
    cthuwu_install_die "--operator is required"
  cthuwu_install_validate_operator "$CTHUWU_INSTALL_OPERATOR"
  [[ -n "$CTHUWU_INSTALL_DIRECTORY" ]] || \
    cthuwu_install_die "the install directory may not be empty"
  [[ "$CTHUWU_INSTALL_DIRECTORY" != *$'\n'* && "$CTHUWU_INSTALL_DIRECTORY" != *$'\r'* ]] || \
    cthuwu_install_die "the install directory may not contain line breaks"
  [[ -n "$CTHUWU_INSTALL_DATA_DIRECTORY" ]] || \
    cthuwu_install_die "the Tentacle data directory may not be empty"
  [[ "$CTHUWU_INSTALL_DATA_DIRECTORY" != *$'\n'* && "$CTHUWU_INSTALL_DATA_DIRECTORY" != *$'\r'* ]] || \
    cthuwu_install_die "the Tentacle data directory may not contain line breaks"
  [[ "$CTHUWU_INSTALL_DIRECTORY" != "$CTHUWU_INSTALL_DATA_DIRECTORY" ]] || \
    cthuwu_install_die "source and Tentacle state require separate directories"
}

cthuwu_install_validate_fresh_paths() {
  if [[ -e "$CTHUWU_INSTALL_DIRECTORY" || -L "$CTHUWU_INSTALL_DIRECTORY" ]]; then
    cthuwu_install_die \
      "the install path already exists; inspect or remove it explicitly: $CTHUWU_INSTALL_DIRECTORY"
  fi
  if [[ -e "$CTHUWU_INSTALL_DATA_DIRECTORY" || -L "$CTHUWU_INSTALL_DATA_DIRECTORY" ]]; then
    cthuwu_install_die \
      "the Tentacle state path already exists; refusing to reuse it: $CTHUWU_INSTALL_DATA_DIRECTORY"
  fi
}

cthuwu_install_prepare_parent() {
  local path="$1"
  local label="$2"
  local parent
  parent="$(dirname -- "$path")"
  [[ ! -L "$parent" ]] || \
    cthuwu_install_die "the $label parent may not be a symbolic link: $parent"
  [[ ! -e "$parent" || -d "$parent" ]] || \
    cthuwu_install_die "the $label parent is not a directory: $parent"
  if [[ ! -e "$parent" && ! -L "$parent" ]]; then
    mkdir -p -- "$parent"
    chmod 700 -- "$parent"
  fi
}

cthuwu_install_build_launcher_arguments() {
  CTHUWU_INSTALL_LAUNCH_ARGS=(
    --data-dir "$CTHUWU_INSTALL_DATA_DIRECTORY"
    --xmtp-env production
    --operator "$CTHUWU_INSTALL_OPERATOR"
  )
}

cthuwu_install_main() {
  umask 077
  [[ "$EUID" != 0 ]] || \
    cthuwu_install_die "do not install or run a Tentacle as root; use a dedicated unprivileged account"
  cthuwu_install_parse_arguments "$@"
  command -v git >/dev/null 2>&1 || \
    cthuwu_install_die "git is required to install Cthuwu"

  cthuwu_install_validate_fresh_paths
  cthuwu_install_prepare_parent "$CTHUWU_INSTALL_DIRECTORY" "install"
  cthuwu_install_prepare_parent "$CTHUWU_INSTALL_DATA_DIRECTORY" "Tentacle state"

  cthuwu_install_log "cloning reviewed source into $CTHUWU_INSTALL_DIRECTORY"
  git clone --depth 1 --branch "$CTHUWU_INSTALL_BRANCH" --single-branch \
    "$CTHUWU_INSTALL_REPOSITORY" "$CTHUWU_INSTALL_DIRECTORY"
  [[ -x "$CTHUWU_INSTALL_DIRECTORY/uwu.sh" ]] || \
    cthuwu_install_die "the cloned checkout has no executable uwu.sh"

  cthuwu_install_log \
    "launching a new production Tentacle in $CTHUWU_INSTALL_DATA_DIRECTORY bound to operator $CTHUWU_INSTALL_OPERATOR"
  cthuwu_install_build_launcher_arguments
  exec "$CTHUWU_INSTALL_DIRECTORY/uwu.sh" "${CTHUWU_INSTALL_LAUNCH_ARGS[@]}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  cthuwu_install_main "$@"
fi
