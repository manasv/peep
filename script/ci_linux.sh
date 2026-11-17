#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
mode="${1:-full}"
case "$mode" in check|full) ;; *) echo "Usage: $0 [check|full]" >&2; exit 2 ;; esac
run() {
  printf '\nRunning:'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}
run cargo fmt --all -- --check
run cargo test --workspace --locked
run cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
run dbus-run-session -- env PEEP_REQUIRE_DBUS_TESTS=1 cargo test -p peep-input --features linux-integration --locked --test linux_bus --test portal_bus
run ./script/test_linux_desktop.sh x11
run ./script/test_linux_desktop.sh wayland
run python3 script/test_inference.py --backend onnx
if [[ "$mode" == full ]]; then run python3 script/package_desktop.py; fi
