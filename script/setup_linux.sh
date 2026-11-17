#!/usr/bin/env bash
set -euo pipefail
privilege=()
if [[ "$(id -u)" != 0 ]]; then privilege=(sudo); fi
"${privilege[@]}" apt-get update
"${privilege[@]}" apt-get install -y \
  build-essential ca-certificates curl git python3 \
  clang cmake pkg-config libasound2-dev libfontconfig1-dev libfreetype6-dev \
  libudev-dev libx11-xcb-dev libxcb-xkb-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libegl1-mesa-dev libvulkan-dev dbus-x11 at-spi2-core \
  libatk-adaptor python3-gi gir1.2-gtk-3.0 xvfb openbox sway wtype
