#!/usr/bin/env bash
set -euo pipefail
mode="${1:-run}"
case "$mode" in
  run|--build|--debug|--logs|--telemetry|--verify) ;;
  *) echo "Usage: $0 [--build|--debug|--logs|--telemetry|--verify]" >&2; exit 2 ;;
esac
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
configuration="${PEEP_CONFIGURATION:-debug}"
case "$configuration" in debug|release) ;; *) echo "Unknown configuration: $configuration" >&2; exit 2 ;; esac
cargo_args=(build --locked -p peep)
if [[ "$configuration" == release ]]; then cargo_args+=(--release); fi
cargo "${cargo_args[@]}"
output="$root/.build/gpui/$configuration"
mkdir -p "$output"
stage="$(mktemp -d "$output/.bundle.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
bundle="$stage/Peep.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$root/target/$configuration/peep" "$bundle/Contents/MacOS/Peep"
cp -f assets/notices/*.txt "$bundle/Contents/Resources/"
cp "$root/LICENSE" "$bundle/Contents/Resources/Peep-LICENSE.txt"
python3 - "$bundle/Contents/Info.plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], 'wb') as f:
    plistlib.dump({
        'CFBundleExecutable': 'Peep', 'CFBundleName': 'Peep',
        'CFBundleDisplayName': 'Peep', 'CFBundleIdentifier': 'dev.peep.app',
        'CFBundlePackageType': 'APPL', 'CFBundleShortVersionString': '0.2.0',
        'CFBundleVersion': '2', 'LSMinimumSystemVersion': '26.0',
        'LSApplicationCategoryType': 'public.app-category.utilities',
        'NSMicrophoneUsageDescription': 'Peep uses your microphone to transcribe your speech on this Mac.',
        'NSHighResolutionCapable': True, 'CFBundleIconFile': 'AppIcon',
    }, f)
PY
iconset="$stage/AppIcon.iconset"
mkdir -p "$iconset"
for points in 16 32 128 256 512; do
  cp "assets/icons/PeepIcon-$points.png" "$iconset/icon_${points}x${points}.png"
  doubled=$((points * 2))
  cp "assets/icons/PeepIcon-$doubled.png" "$iconset/icon_${points}x${points}@2x.png"
done
/usr/bin/iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/AppIcon.icns"
identity="${PEEP_SIGN_IDENTITY:--}"
sign_args=(--force --sign "$identity" --options runtime)
if [[ "$identity" != - ]]; then sign_args+=(--timestamp); fi
/usr/bin/codesign "${sign_args[@]}" --entitlements "$root/Peep.entitlements" "$bundle"
/usr/bin/codesign --verify --deep --strict "$bundle"
app="$output/Peep.app"
if [[ -e "$app" ]]; then mv "$app" "$stage/previous.app"; fi
mv "$bundle" "$app"
if [[ "$mode" == --build ]]; then echo "$app"; exit 0; fi
models="$root/.build/models/scriptrs"
args=()
if [[ -d "$models/parakeet-v3" ]]; then args=(--parakeet-models "$models"); fi
case "$mode" in
  --debug) lldb -- "$app/Contents/MacOS/Peep" "${args[@]}" ;;
  --logs|--telemetry)
    "$app/Contents/MacOS/Peep" "${args[@]}"
    ;;
  --verify)
    /usr/bin/open "$app" --args "${args[@]}"
    sleep 1
    pgrep -x Peep
    ;;
  run) /usr/bin/open "$app" --args "${args[@]}" ;;
esac
