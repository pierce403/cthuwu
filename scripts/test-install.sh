#!/usr/bin/env bash

set -Eeuo pipefail

readonly TEST_INSTALL_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly TEST_INSTALL_REPO_ROOT="$(cd -- "$TEST_INSTALL_SCRIPT_DIR/.." && pwd -P)"

# shellcheck source=../install.sh
source "$TEST_INSTALL_REPO_ROOT/install.sh"

install_test_fail() {
  printf 'install.sh test failed: %s\n' "$*" >&2
  exit 1
}

install_test_equal() {
  [[ "$1" == "$2" ]] || install_test_fail "$3 (expected '$1', got '$2')"
}

install_test_rejected() {
  local label="$1"
  shift
  if ("$@") >/dev/null 2>&1; then
    install_test_fail "$label was accepted"
  fi
}

bash -n "$TEST_INSTALL_REPO_ROOT/install.sh"

operator="0x1111111111111111111111111111111111111111"
cthuwu_install_validate_operator "$operator"
install_test_rejected "short operator address" cthuwu_install_validate_operator 0x1234
install_test_rejected "zero operator address" \
  cthuwu_install_validate_operator 0x0000000000000000000000000000000000000000

CTHUWU_INSTALL_DIR="/tmp/cthuwu source"
CTHUWU_INSTALL_DATA_DIR="/tmp/cthuwu fresh state"
export CTHUWU_INSTALL_DIR
export CTHUWU_INSTALL_DATA_DIR
cthuwu_install_parse_arguments --operator "$operator"
install_test_equal "$operator" "$CTHUWU_INSTALL_OPERATOR" "operator forwarding"
install_test_equal "/tmp/cthuwu source" "$CTHUWU_INSTALL_DIRECTORY" "environment install path"
install_test_equal "/tmp/cthuwu fresh state" "$CTHUWU_INSTALL_DATA_DIRECTORY" "environment state path"

cthuwu_install_parse_arguments \
  --operator="$operator" \
  --install-dir "/tmp/explicit source" \
  --data-dir "/tmp/explicit fresh state"
install_test_equal "/tmp/explicit source" "$CTHUWU_INSTALL_DIRECTORY" "CLI install path"
install_test_equal "/tmp/explicit fresh state" "$CTHUWU_INSTALL_DATA_DIRECTORY" "CLI state path"

cthuwu_install_build_launcher_arguments
install_test_equal "4" "${#CTHUWU_INSTALL_LAUNCH_ARGS[@]}" "launcher argument count"
install_test_equal "--data-dir" "${CTHUWU_INSTALL_LAUNCH_ARGS[0]}" "data flag"
install_test_equal "/tmp/explicit fresh state" "${CTHUWU_INSTALL_LAUNCH_ARGS[1]}" "fresh state forwarding"
install_test_equal "--operator" "${CTHUWU_INSTALL_LAUNCH_ARGS[2]}" "operator flag"
install_test_equal "$operator" "${CTHUWU_INSTALL_LAUNCH_ARGS[3]}" "public operator forwarding"

CTHUWU_INSTALL_DIRECTORY="/tmp/cthuwu-installer-source-does-not-exist"
CTHUWU_INSTALL_DATA_DIRECTORY="/tmp"
install_test_rejected "existing Tentacle state" cthuwu_install_validate_fresh_paths

install_test_rejected "missing operator" cthuwu_install_parse_arguments
install_test_rejected "unknown argument" \
  cthuwu_install_parse_arguments --operator "$operator" --private-key nope

printf 'install.sh tests passed\n'
