#!/usr/bin/env bash
# What a build of the APK needs, for .github/actions/android-toolchain (CI
# and release):
#
#   scripts/ci/android-toolchain.sh versions   the JDK's and Gradle's versions
#                                              as step outputs (java, gradle)
#   scripts/ci/android-toolchain.sh install    Rust with the Android targets,
#                                              cargo-ndk, the NDK; sets
#                                              ANDROID_NDK_HOME for the job
#
# The versions: scripts/ci/versions.env, and the NDK's in
# android/gradle.properties (amp.ndkVersion). The JDK and Gradle themselves
# come from their setup actions (pinned in the composite action).
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
. scripts/ci/lib.sh
. scripts/ci/versions.env

ndk_version() {
    local version
    version=$(sed -n 's/^amp\.ndkVersion=//p' android/gradle.properties | tr -d '[:space:]')
    if [ -z "$version" ]; then
        echo "::error::no amp.ndkVersion in android/gradle.properties"
        exit 1
    fi
    printf '%s' "$version"
}

versions() {
    {
        echo "java=$JAVA_VERSION"
        echo "gradle=$GRADLE_VERSION"
        echo "ndk=$(ndk_version)"
        echo "cargo-ndk=$CARGO_NDK_VERSION"
    } | tee -a "${GITHUB_OUTPUT:-/dev/null}"
}

rust() {
    # (the version rust-toolchain.toml pins, with its targets: every cargo
    # in this checkout uses it)
    local channel
    channel=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
    [ -n "$channel" ] || { echo "::error::no channel in rust-toolchain.toml"; exit 1; }
    rustup toolchain install "$channel" --profile minimal --component clippy --component rustfmt \
        --target aarch64-linux-android --target x86_64-linux-android
    rustup default "$channel"
    rustc --version
    cargo --version
}

cargo_ndk() {
    if cargo ndk --version 2>/dev/null | grep -q -F "$CARGO_NDK_VERSION"; then
        echo "cargo-ndk $CARGO_NDK_VERSION is there"
    else
        cargo install cargo-ndk --version "$CARGO_NDK_VERSION" --locked
    fi
    cargo ndk --version
}

sdkmanager_path() {
    local found
    for found in "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
        $(ls -d "$ANDROID_HOME"/cmdline-tools/*/bin/sdkmanager 2>/dev/null | sort -V -r); do
        if [ -x "$found" ]; then
            printf '%s' "$found"
            return 0
        fi
    done
    echo "::error::no sdkmanager under $ANDROID_HOME/cmdline-tools"
    return 1
}

# sdkmanager PACKAGE...: installed, the licences accepted (the runner's
# image has accepted them; a new one is accepted here). Its progress bars
# are left out of the log.
sdk_install() {
    local sdkmanager
    sdkmanager=$(sdkmanager_path)
    # (`yes` ends with SIGPIPE when sdkmanager is done: not a failure)
    { yes 2>/dev/null || true; } | "$sdkmanager" --sdk_root="$ANDROID_HOME" --install "$@" \
        | tr '\r' '\n' | grep -v -E '^\s*\[=*\s*\]|^\s*$' || true
    # (sdkmanager's exit status is not to be relied on: what counts is
    # whether the package is there afterwards, which the caller checks)
}

ndk() {
    local version path
    version=$(ndk_version)
    : "${ANDROID_HOME:?ANDROID_HOME is not set: the Android SDK of the runner}"
    path="$ANDROID_HOME/ndk/$version"
    if [ -f "$path/build/cmake/android.toolchain.cmake" ]; then
        echo "the NDK $version is in the runner's image"
    else
        echo "installing the NDK $version (the image has: $(ls "$ANDROID_HOME/ndk" 2>/dev/null | tr '\n' ' '))"
        sdk_install "ndk;$version"
    fi
    if [ ! -f "$path/build/cmake/android.toolchain.cmake" ]; then
        echo "::error::the NDK $version is not at $path after sdkmanager ran"
        ls -la "$ANDROID_HOME/ndk" || true
        return 1
    fi
    grep -E '^Pkg.Revision' "$path/source.properties" || true
    # (cargo-ndk, libopus's CMake build and Gradle look at these; the image
    # points them at its default NDK)
    {
        echo "ANDROID_NDK_HOME=$path"
        echo "ANDROID_NDK_ROOT=$path"
        echo "ANDROID_NDK=$path"
    } >> "${GITHUB_ENV:-/dev/null}"
    export ANDROID_NDK_HOME=$path ANDROID_NDK_ROOT=$path ANDROID_NDK=$path
    echo "the NDK: $path"
}

others() {
    # (libopus is built with CMake; Gradle's own build needs nothing more)
    cmake --version | head -1
    java -version 2>&1 | head -1
    if command -v gradle >/dev/null; then gradle --version | grep -E '^Gradle' || true; fi
    ls "$ANDROID_HOME/platforms" | tr '\n' ' '; echo
    ls "$ANDROID_HOME/build-tools" | tr '\n' ' '; echo
}

case "${1:-}" in
    versions)
        versions
        ;;
    install)
        step "Rust (stable) with the Android targets" rust
        step "cargo-ndk $CARGO_NDK_VERSION" cargo_ndk
        step "The NDK $(ndk_version)" ndk
        # (the step ran ndk in a pipe: its exports are not here, GITHUB_ENV has them)
        step "CMake, the JDK, Gradle, the SDK's platforms" others
        ;;
    *)
        echo "usage: $0 versions|install" >&2
        exit 2
        ;;
esac
