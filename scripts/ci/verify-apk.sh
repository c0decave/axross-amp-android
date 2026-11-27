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

# No network permission: the app makes no connection at all, and without
# INTERNET Android does not let it open a socket (the merged manifest, as
# the APK carries it).
AAPT2=$(ls -d "$ANDROID_HOME"/build-tools/*/aapt2 2>/dev/null | sort -V | tail -1)
if [ -n "$AAPT2" ]; then
    permissions=$("$AAPT2" dump permissions "$APK" 2>&1)
    printf '%s\n' "$permissions"
    if printf '%s\n' "$permissions" | grep -E -q 'android\.permission\.(INTERNET|ACCESS_NETWORK_STATE|ACCESS_WIFI_STATE|CHANGE_NETWORK_STATE|CHANGE_WIFI_STATE)'; then
        printf '%s\n' "$permissions" | annotate error "a network permission in $(basename "$APK")"
        exit 1
    fi
    echo "no network permission"
else
    echo "::warning::no aapt2: the APK's permissions are not checked"
fi

# 16 KB pages (Android 15 on some devices): every native library's LOAD
# segments aligned to 16 KB at least, and stored page-aligned in the APK -
# else UnsatisfiedLinkError at the start on such a device.
READELF=${ANDROID_NDK_HOME:+$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf}
if [ -n "$READELF" ] && [ -x "$READELF" ]; then
    so_dir=$(mktemp -d)
    for so in $(printf '%s\n' "$libs" | awk '{print $NF}'); do
        unzip -p "$APK" "$so" > "$so_dir/lib.so"
        bad=""
        for align in $("$READELF" -lW "$so_dir/lib.so" | awk '$1 == "LOAD" { print $NF }'); do
            [ $((align)) -ge 16384 ] || bad="$bad $align"
        done
        if [ -n "$bad" ]; then
            echo "$so: LOAD segments aligned to $bad" | annotate error "not 16 KB aligned: $so"
            exit 1
        fi
        echo "$so: LOAD segments 16 KB aligned"
    done
    rm -rf "$so_dir"
else
    echo "::warning::no llvm-readelf (ANDROID_NDK_HOME): the 16 KB alignment of the libraries is not checked"
fi
ZIPALIGN=$(ls -d "$ANDROID_HOME"/build-tools/*/zipalign 2>/dev/null | sort -V | tail -1)
if [ -n "$ZIPALIGN" ]; then
    if ! out=$("$ZIPALIGN" -c -P 16 -v 4 "$APK" 2>&1); then
        printf '%s\n' "$out" | grep -v -E '\(OK' | tail -n 30 | annotate error "zipalign: not 16 KB page-aligned"
        exit 1
    fi
    echo "zipalign: page-aligned for 16 KB"
fi

{
    grep -E '^Verified using|^Verifies' "$verify" || true
    echo "certificate SHA-256 $GOT"
} | annotate notice "$(basename "$APK") is signed"
echo "certificate: $GOT"
