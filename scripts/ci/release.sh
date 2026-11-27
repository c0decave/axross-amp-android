#!/usr/bin/env bash
# The release's APK (release.yml, job apk), after the toolchain:
#
#   scripts/ci/release.sh build    the arm64-v8a release APK, signed with the
#                                  key in AMP_KEYSTORE_FILE (AMP_KEYSTORE_PASSWORD,
#                                  AMP_KEY_ALIAS, AMP_KEY_PASSWORD), and the
#                                  arm64-v8a debug APK from the same commit
#   scripts/ci/release.sh verify   the release APK's signature by the certificate
#                                  in AMP_CERT_SHA256 (where one is named), both
#                                  APKs' libraries, and the files to publish:
#                                  release/axross-amp-android-$VERSION-arm64-v8a.apk
#                                  release/axross-amp-android-$VERSION-arm64-v8a-debug.apk
#
# The debug APK (debuggable, signed with the build's debug key, the package
# io.github.c0decave.axrossamp.debug beside the release's, more in its log)
# goes into the release with the release APK: the publish step takes every
# file in release/, and SHA256SUMS lists them all.
#
# Every failure leaves an annotation saying why (scripts/ci/lib.sh).
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
. scripts/ci/lib.sh

BUILT=android/app/build/outputs/apk/release/app-release.apk
BUILT_DEBUG=android/app/build/outputs/apk/debug/app-debug.apk

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
    # (the release's signing variables are not used for it: Gradle signs a
    # debug build with its own debug key)
    step "Debug APK (arm64-v8a), from the same commit" \
        gradle_retrying -p android -Pabis=arm64-v8a assembleDebug
    if [ ! -f "$BUILT_DEBUG" ]; then
        ls -l android/app/build/outputs/apk/debug 2>&1 \
            | annotate error "no debug APK (app-debug.apk) was built"
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
    step "The debug APK's signature and library (scripts/ci/verify-apk.sh)" \
        bash scripts/ci/verify-apk.sh "$BUILT_DEBUG" "" arm64-v8a
    mkdir -p release
    cp "$BUILT" "release/axross-amp-android-$VERSION-arm64-v8a.apk"
    cp "$BUILT_DEBUG" "release/axross-amp-android-$VERSION-arm64-v8a-debug.apk"
    ls -l release
    sha256sum release/*.apk | annotate notice "The release's APKs"
}

case "${1:-}" in
    build) build ;;
    verify) verify ;;
    *) echo "usage: $0 build|verify" >&2; exit 2 ;;
esac
