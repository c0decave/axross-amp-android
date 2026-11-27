#!/usr/bin/env bash
# The release's APK (release.yml, job apk), after the toolchain:
#
#   scripts/ci/release.sh build    the arm64-v8a release APK, signed with the
#                                  key in AMP_KEYSTORE_FILE (AMP_KEYSTORE_PASSWORD,
#                                  AMP_KEY_ALIAS, AMP_KEY_PASSWORD)
#   scripts/ci/release.sh verify   its signature by the certificate in
#                                  AMP_CERT_SHA256 (where one is named), its
#                                  library, and the file to publish:
#                                  release/axross-amp-android-$VERSION-arm64-v8a.apk
#
# Every failure leaves an annotation saying why (scripts/ci/lib.sh).
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
. scripts/ci/lib.sh

BUILT=android/app/build/outputs/apk/release/app-release.apk

build() {
    for name in AMP_KEYSTORE_FILE AMP_KEYSTORE_PASSWORD AMP_KEY_ALIAS AMP_KEY_PASSWORD; do
        [ -n "${!name:-}" ] || { echo "::error::$name is not set: no signed release"; exit 1; }
    done
    step "Release APK (arm64-v8a), signed" \
        gradle_retrying -p android -Pabis=arm64-v8a assembleRelease
    if [ ! -f "$BUILT" ]; then
        ls -l android/app/build/outputs/apk/release 2>&1 \
            | annotate error "no signed APK (app-release.apk) was built"
        exit 1
    fi
}

verify() {
    : "${VERSION:?VERSION is not set}"
    local expected=${AMP_CERT_SHA256:-}
    if [ -z "$expected" ]; then
        echo "::warning::no AMP_CERT_SHA256 variable: the certificate is not checked against an expected one"
    fi
    step "The APK's signature and library (scripts/ci/verify-apk.sh)" \
        bash scripts/ci/verify-apk.sh "$BUILT" "$expected" arm64-v8a
    mkdir -p release
    cp "$BUILT" "release/axross-amp-android-$VERSION-arm64-v8a.apk"
    ls -l release
    sha256sum release/*.apk | annotate notice "The release APK"
}

case "${1:-}" in
    build) build ;;
    verify) verify ;;
    *) echo "usage: $0 build|verify" >&2; exit 2 ;;
esac
