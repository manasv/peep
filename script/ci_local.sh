#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
mode="${1:-full}"
case "$mode" in check|full) ;; *) echo "Usage: $0 [check|full]" >&2; exit 2 ;; esac
mkdir -p .build/ci
log="$root/.build/ci/linux-$(date -u +%Y%m%dT%H%M%SZ).log"
printf 'Linux CI log: %s\n' "$log"
if [[ "$(uname -s)" == Linux ]]; then
  ./script/ci_linux.sh "$mode" 2>&1 | tee "$log"
  exit
fi
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'Run this script on Linux or macOS.' >&2
  exit 2
fi
if ! command -v limactl >/dev/null; then
  echo 'Install Lima first: brew install lima' >&2
  exit 2
fi
vm="${PEEP_CI_VM:-peep-ci}"
if ! limactl list --format '{{.Name}}' | awk -v name="$vm" '$0 == name { found=1 } END { exit !found }'; then
  limactl start --tty=false --name="$vm" --vm-type=vz --cpus=6 --memory=16 \
    --disk=80 --containerd=none --mount-only="$root:w" template:ubuntu-24.04
else
  limactl start --tty=false "$vm"
fi
limactl shell "$vm" bash -s -- "$root" "$mode" <<'GUEST' 2>&1 | tee "$log"
set -euo pipefail
cd "$1"
export PATH="$HOME/.cargo/bin:$PATH"
toolchain=1.96.0
cache="${XDG_CACHE_HOME:-$HOME/.cache}/peep-ci"
mkdir -p "$cache"
setup="$(sha256sum script/setup_linux.sh | cut -d' ' -f1)"
if [[ ! -e "$cache/setup-$setup" ]]; then
  ./script/setup_linux.sh
  touch "$cache/setup-$setup"
fi
if ! command -v rustup >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$toolchain"
fi
rustup toolchain install "$toolchain" --profile minimal --component rustfmt --component clippy
export RUSTUP_TOOLCHAIN="$toolchain"
export CARGO_TARGET_DIR="$cache/target-$(uname -m)"
export CARGO_BUILD_JOBS=4
./script/ci_linux.sh "$2"
GUEST
