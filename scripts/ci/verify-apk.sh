#!/usr/bin/env bash
# An APK's signature and contents, checked (the release; the CI's debug APK
# as a rehearsal):
#
#   scripts/ci/verify-apk.sh APK [EXPECTED_SHA256] [ABI...]
#
# - apksigner (the newest of the SDK's build-tools that runs with this JDK)
#   verifies the signature and names the certificate;
# - with EXPECTED_SHA256 (colons and case do not matter) the certificate
#   must be that one;
# - every ABI named must have lib/<abi>/libaxrossamp.so in the APK.
#
# Prints the certificate's SHA-256 as the last line of its output
# ("certificate: <hex>"); every failure is an annotation saying why.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
. "$ROOT/scripts/ci/lib.sh"

APK=${1:?the APK}
EXPECTED=${2:-}
shift $(( $# >= 2 ? 2 : $# ))
ABIS=("$@")

fail() {
    echo "::error title=APK check::$*"
    exit 1
}

[ -f "$APK" ] || fail "no APK at $APK"
: "${ANDROID_HOME:?ANDROID_HOME is not set}"

# The newest apksigner that runs: a newer build-tools may want a newer JDK.
APKSIGNER=""
tried=""
for candidate in $(ls -d "$ANDROID_HOME"/build-tools/*/apksigner 2>/dev/null | sort -V -r); do
    if out=$("$candidate" --version 2>&1); then
        APKSIGNER=$candidate
        echo "apksigner $out ($candidate)"
        break
    fi
    tried="$tried
$candidate: $(printf '%s' "$out" | head -n 3)"
done
if [ -n "$tried" ]; then
    printf 'java: %s%s\n' "$(java -version 2>&1 | head -1)" "$tried" \
        | annotate warning "apksigner that did not run"
fi
[ -n "$APKSIGNER" ] || fail "no apksigner under $ANDROID_HOME/build-tools runs"

verify=$(mktemp)
if ! "$APKSIGNER" verify --verbose --print-certs "$APK" > "$verify" 2>&1; then
    annotate error "apksigner verify failed" < "$verify"
    cat "$verify"
    exit 1
fi
cat "$verify"
GOT=$(grep -m1 -E 'certificate SHA-256 digest: ' "$verify" | sed 's/.*certificate SHA-256 digest: //' | tr -d ' \r' | tr 'A-F' 'a-f')
if [ -z "$GOT" ]; then
    annotate error "no certificate digest in apksigner's output" < "$verify"
    exit 1
fi

if [ -n "$EXPECTED" ]; then
    WANT=$(printf '%s' "$EXPECTED" | tr -d ': \r\n' | tr 'A-F' 'a-f')
    [ "$GOT" = "$WANT" ] || fail "signed by the certificate $GOT, expected $WANT"
    echo "the expected certificate"
fi

libs=$(unzip -l "$APK" | grep -E 'lib/[^/]+/[^/]+\.so$' || true)
for abi in "${ABIS[@]}"; do
    if ! printf '%s\n' "$libs" | grep -q "lib/$abi/libaxrossamp.so"; then
        printf 'the native libraries in the APK:\n%s\n' "${libs:-(none)}" \
            | annotate error "no lib/$abi/libaxrossamp.so in $(basename "$APK")"
        exit 1
    fi
done
printf '%s\n' "$libs"

{
    grep -E '^Verified using|^Verifies' "$verify" || true
    echo "certificate SHA-256 $GOT"
} | annotate notice "$(basename "$APK") is signed"
echo "certificate: $GOT"
