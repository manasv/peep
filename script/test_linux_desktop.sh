#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
mode="${1:-x11}"
case "$mode" in x11|wayland) ;; *) echo 'Usage: test_linux_desktop.sh [x11|wayland]' >&2; exit 2 ;; esac
if [[ "${PEEP_TEST_PRIVATE_BUS:-}" != 1 ]]; then
  exec dbus-run-session -- env PEEP_TEST_PRIVATE_BUS=1 "$0" "$mode"
fi
if [[ "$mode" == x11 && "${PEEP_TEST_PRIVATE_DISPLAY:-}" != 1 ]]; then
  exec xvfb-run -a env PEEP_TEST_PRIVATE_DISPLAY=1 "$0" x11
fi
runtime="$(mktemp -d)"
chmod 700 "$runtime"
processes=()
cleanup() {
  for process in "${processes[@]}"; do kill "$process" 2>/dev/null || true; done
  for process in "${processes[@]}"; do wait "$process" 2>/dev/null || true; done
  rm -rf -- "$runtime"
}
trap cleanup EXIT
if [[ "$mode" == x11 ]]; then
  unset WAYLAND_DISPLAY
  export XDG_SESSION_TYPE=x11 GDK_BACKEND=x11
  openbox > "$runtime/compositor.log" 2>&1 &
  processes+=("$!")
else
  unset DISPLAY
  export XDG_RUNTIME_DIR="$runtime"
  export XDG_SESSION_TYPE=wayland GDK_BACKEND=wayland
  export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
  cat > "$runtime/sway.conf" <<'CONFIG'
output * resolution 1024x768
seat seat0 fallback true
default_border none
focus_follows_mouse no
CONFIG
  sway --config "$runtime/sway.conf" > "$runtime/compositor.log" 2>&1 &
  processes+=("$!")
  for _ in {1..100}; do
    socket="$(find "$runtime" -maxdepth 1 -type s -name 'wayland-*' -print -quit)"
    [[ -n "$socket" ]] && break
    sleep 0.1
  done
  if [[ -z "${socket:-}" ]]; then cat "$runtime/compositor.log" >&2; exit 1; fi
  export WAYLAND_DISPLAY="${socket##*/}"
  wtype -s 600000 > "$runtime/input.log" 2>&1 &
  processes+=("$!")
fi
PEEP_TEST_LINUX_EDITOR=1 cargo test -p peep-input --locked --test linux_editor -- --nocapture
