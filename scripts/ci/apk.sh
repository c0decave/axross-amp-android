#!/usr/bin/env bash
# The CI's debug APK (ci.yml, job apk), after scripts/ci/android-toolchain.sh:
#
#   1. the Rust side type-checks for Android without a warning (clippy);
#   2. Gradle builds the debug APK (it runs cargo-ndk for arm64-v8a and
#      x86_64 itself), and the APK holds both libraries;
#   3. while android/gradle/verification-metadata.xml is not committed,
#      Gradle writes the file it would be (offered as an artifact to review;
#      docs/ARCHITECTURE.md, "Supply chain").
#
# A failing step leaves the reason as an annotation of the job
# (scripts/ci/lib.sh).
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
. scripts/ci/lib.sh

APK=android/app/build/outputs/apk/debug/app-debug.apk
GRADLE=(gradle --no-daemon --console=plain --stacktrace -p android)

clippy() {
    cargo ndk --target arm64-v8a --platform 26 clippy --locked --lib -- -D warnings
}

inspect() {
    ls -l "$APK"
    unzip -l "$APK" | grep -E 'lib/|classes|AndroidManifest|assets/'
    unzip -l "$APK" | grep -q 'lib/arm64-v8a/libaxrossamp.so'
    unzip -l "$APK" | grep -q 'lib/x86_64/libaxrossamp.so'
    sha256sum "$APK"
}

step "The Rust side for Android (clippy, no warnings)" clippy
step "Debug APK (arm64-v8a and x86_64)" "${GRADLE[@]}" assembleDebug
step "What is in the APK" inspect
{
    echo "$(basename "$APK"): $(stat -c %s "$APK") bytes, sha256 $(sha256sum "$APK" | cut -d' ' -f1)"
    unzip -l "$APK" | grep -E 'lib/.*\.so' || true
} | annotate notice "The debug APK"

if [ ! -f android/gradle/verification-metadata.xml ]; then
    step "Gradle's dependency checksums (to review and commit)" \
        "${GRADLE[@]}" -PskipCargo --write-verification-metadata sha256 assembleDebug
fi
