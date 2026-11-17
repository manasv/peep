#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./script/package_release.sh [--output-dir DIRECTORY]
       ./script/package_release.sh [--output-dir DIRECTORY] \
         --identity DEVELOPER_ID_SHA1 --notary-profile KEYCHAIN_PROFILE

With no signing options, build a locally testable ad hoc Release ZIP.
With both signing options, sign with Developer ID and notarize with Apple.
The default destination is .build/release. Existing ZIPs are never replaced.
EOF
}

fail() {
  printf 'package_release: %s\n' "$*" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$root/.build/release"
identity=""
notary_profile=""

while (($#)); do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --output-dir|--identity|--notary-profile)
      [[ $# -ge 2 && -n "$2" && "$2" != -* ]] || fail "Missing value for $1."
      case "$1" in
        --output-dir) output="$2" ;;
        --identity) identity="$2" ;;
        --notary-profile) notary_profile="$2" ;;
      esac
      shift 2
      ;;
    *) fail "Unknown argument '$1'. Use --help for usage." ;;
  esac
done

if [[ -n "$identity" && -z "$notary_profile" || -z "$identity" && -n "$notary_profile" ]]; then
  fail "Supply --identity and --notary-profile together."
fi

variant="local"
if [[ -n "$identity" ]]; then
  [[ "$identity" =~ ^[[:xdigit:]]{40}$ ]] || fail "--identity expects the 40-digit Developer ID Application SHA-1 fingerprint."
  identity="$(printf '%s' "$identity" | tr '[:lower:]' '[:upper:]')"
  identities="$(/usr/bin/security find-identity -v -p codesigning)" ||
    fail "Could not read the Keychain's signing identities."
  identity_name="$(
    printf '%s\n' "$identities" |
      awk -v fingerprint="$identity" '
        $2 == fingerprint && /"Developer ID Application: / {
          sub(/^[^"]*"/, ""); sub(/"$/, ""); print
        }'
  )"
  [[ "$identity_name" =~ \(([A-Z0-9]{10})\)$ ]] ||
    fail "No valid Developer ID Application identity matches that fingerprint. Install its certificate and private key first."
  team="${BASH_REMATCH[1]}"
  variant="notarized"
fi

mkdir -p -- "$output"
output="$(cd -- "$output" && pwd -P)"
stage="$(mktemp -d "$output/.package.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT

PEEP_CONFIGURATION=release PEEP_SIGN_IDENTITY="${identity:--}" "$root/script/build_and_run.sh" --build
product="$root/.build/gpui/release/Peep.app"
[[ -x "$product/Contents/MacOS/Peep" ]] || fail "Could not find the built Peep executable."
version="$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - "$product/Contents/Info.plist")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "Unexpected release version '$version'."
microphone_usage="$(/usr/bin/plutil -extract NSMicrophoneUsageDescription raw -o - "$product/Contents/Info.plist" 2>/dev/null || true)"
[[ -n "$microphone_usage" ]] || fail "Info.plist is missing the microphone usage description."
archive="$output/Peep-$version-$variant.zip"
[[ ! -e "$archive" && ! -L "$archive" ]] || fail "Archive already exists at $archive. Use another output directory."

app="$stage/Peep.app"
/usr/bin/ditto "$product" "$app"
/usr/bin/codesign --verify --deep --strict "$app"
signing_details="$(/usr/bin/codesign --display --verbose=4 "$app" 2>&1)"
[[ "$signing_details" =~ flags=.*[\(,]runtime[,\)] ]] || fail "The app signature does not enable Hardened Runtime."

/usr/bin/codesign --display --entitlements - --xml "$app" > "$stage/entitlements.plist"
microphone="$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.device.audio-input' "$stage/entitlements.plist" 2>/dev/null || true)"
[[ "$microphone" == "true" ]] || fail "The app signature is missing the microphone entitlement."
if /usr/libexec/PlistBuddy -c 'Print :com.apple.security.get-task-allow' "$stage/entitlements.plist" >/dev/null 2>&1; then
  fail "The app signature contains the debug get-task-allow entitlement."
fi

if [[ "$variant" == "notarized" ]]; then
  [[ "$signing_details" == *"Authority=Developer ID Application: "* &&
     "$signing_details" == *"TeamIdentifier=$team"* &&
     "$signing_details" == *"Timestamp="* ]] || fail "The app lacks the selected Developer ID signature and secure timestamp."
  /usr/bin/codesign --verify --test-requirement "=certificate leaf = H\"$identity\"" "$app"

  /usr/bin/ditto -c -k --keepParent "$app" "$stage/submission.zip"
  /usr/bin/xcrun notarytool submit "$stage/submission.zip" --keychain-profile "$notary_profile" \
    --output-format json --no-progress > "$stage/submission.json"
  submission="$(/usr/bin/plutil -extract id raw -o - "$stage/submission.json")"
  [[ "$submission" =~ ^[[:xdigit:]]{8}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{12}$ ]] ||
    fail "Apple returned an invalid notarization submission ID."
  printf 'Notarization submission: %s\n' "$submission"
  wait_failed=0
  /usr/bin/xcrun notarytool wait "$submission" --keychain-profile "$notary_profile" \
    --output-format json > "$stage/notary-status.json" || wait_failed=1
  status="$(/usr/bin/plutil -extract status raw -o - "$stage/notary-status.json" 2>/dev/null || true)"
  if [[ -z "$status" && "$wait_failed" == 1 ]]; then
    fail "Could not finish waiting for notarization. Check submission $submission with notarytool info."
  fi
  if [[ "$status" != "Accepted" ]]; then
    /usr/bin/xcrun notarytool log "$submission" --keychain-profile "$notary_profile" >&2 || true
    fail "Apple returned '$status' for notarization submission $submission."
  fi
  /usr/bin/xcrun stapler staple "$app"
fi

/usr/bin/ditto -c -k --keepParent "$app" "$stage/release.zip"
mkdir "$stage/unpacked"
/usr/bin/ditto -x -k "$stage/release.zip" "$stage/unpacked"
unpacked="$stage/unpacked/Peep.app"
/usr/bin/codesign --verify --deep --strict "$unpacked"
if [[ "$variant" == "notarized" ]]; then
  /usr/bin/xcrun stapler validate "$unpacked"
  /usr/sbin/spctl --assess --type execute --verbose=4 "$unpacked"
fi

/bin/ln -h "$stage/release.zip" "$archive" || fail "Could not save $archive without replacing an existing file."
printf 'Release archive: %s\n' "$archive"
/usr/bin/shasum -a 256 "$archive"
