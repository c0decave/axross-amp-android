#!/usr/bin/env bash
# Is the release APK reproducible? The unsigned release APK (arm64-v8a) of
# this commit, built from clean clones in other folders - as F-Droid's
# build server would build it - compared:
#
#   scripts/ci/reproducible.sh twice           two builds, in two different
#                                              folders with two Cargo homes:
#                                              byte for byte the same?
#   scripts/ci/reproducible.sh against APK     one build, compared with the
#                                              signed APK the way F-Droid does
#                                              (apksigcopier compare: the same
#                                              but for the signature?)
#
# Needs what a build needs (scripts/ci/android-toolchain.sh). Every
# difference is an annotation: which files in the APK differ and, for the
# native library, the strings that differ (paths, mostly).
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
. scripts/ci/lib.sh

WORK=${RUNNER_TEMP:-/tmp}/reproducible
UNSIGNED=android/app/build/outputs/apk/release/app-release-unsigned.apk

# A clean clone of this commit (the submodule from this checkout's copy:
# no network) in DIR, built with CARGO_HOME; the unsigned APK to OUT.
build_in() {
    local dir=$1 cargo_home=$2 out=$3
    rm -rf "$dir"
    mkdir -p "$(dirname "$dir")" "$cargo_home"
    git clone -q --no-hardlinks "$ROOT" "$dir"
    git -C "$dir" checkout -q "$(git rev-parse HEAD)"
    git -C "$dir" config submodule.axross-amp.url "$ROOT/axross-amp"
    git -C "$dir" -c protocol.file.allow=always submodule update -q --init
    # (the downloaded crates shared, the builds not: only the registry's
    # cache is copied in, the paths are this Cargo home's)
    if [ -d "${CARGO_HOME:-$HOME/.cargo}/registry/cache" ] && [ "$cargo_home" != "${CARGO_HOME:-$HOME/.cargo}" ]; then
        mkdir -p "$cargo_home/registry"
        cp -r "${CARGO_HOME:-$HOME/.cargo}/registry/cache" "${CARGO_HOME:-$HOME/.cargo}/registry/index" "$cargo_home/registry/" 2>/dev/null || true
    fi
    (
        cd "$dir"
        unset AMP_KEYSTORE_FILE AMP_KEYSTORE_PASSWORD AMP_KEY_ALIAS AMP_KEY_PASSWORD
        export CARGO_HOME=$cargo_home
        export SOURCE_DATE_EPOCH
        SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
        gradle_retrying -p android -Pabis=arm64-v8a assembleRelease
    )
    cp "$dir/$UNSIGNED" "$out"
    echo "$(basename "$out"): $(sha256sum "$out" | cut -d' ' -f1) (built in $dir)"
}

# What differs between two APKs: their files, and the native library's strings.
explain() {
    local a=$1 b=$2
    python3 -I - "$a" "$b" <<'PY'
import hashlib, sys, zipfile
a, b = (zipfile.ZipFile(p) for p in sys.argv[1:3])
na, nb = a.namelist(), b.namelist()
if na != nb:
    print("the files or their order differ:")
    print("  only in the first:", sorted(set(na) - set(nb)))
    print("  only in the second:", sorted(set(nb) - set(na)))
for name in na:
    if name in nb:
        x, y = a.read(name), b.read(name)
        ia, ib = a.getinfo(name), b.getinfo(name)
        if x != y:
            print(f"differs: {name} ({len(x)} / {len(y)} bytes)")
        elif (ia.date_time, ia.extra, ia.compress_type, ia.external_attr) != (ib.date_time, ib.extra, ib.compress_type, ib.external_attr):
            print(f"same content, other zip fields: {name}")
PY
    local tmp
    tmp=$(mktemp -d)
    (cd "$tmp" && mkdir a b && unzip -q -o "$a" 'lib/*' -d a && unzip -q -o "$b" 'lib/*' -d b) || true
    for so in "$tmp"/a/lib/*/*.so; do
        local other=$tmp/b/${so#"$tmp"/a/}
        [ -f "$other" ] || continue
        if ! cmp -s "$so" "$other"; then
            echo "strings that differ in ${so#"$tmp"/a/} (first 40):"
            diff <(strings -n 8 "$so" | sort -u) <(strings -n 8 "$other" | sort -u) | head -n 40 || true
        fi
    done
    rm -rf "$tmp"
}

twice() {
    mkdir -p "$WORK"
    step "Build 1 (a clean clone in $WORK/one)" \
        build_in "$WORK/one/axross-amp-android" "$WORK/one/cargo" "$WORK/first.apk"
    step "Build 2 (another folder, another Cargo home)" \
        build_in "$WORK/elsewhere/deeper/two/src" "$WORK/elsewhere/cargo-two" "$WORK/second.apk"
    if cmp -s "$WORK/first.apk" "$WORK/second.apk"; then
        sha256sum "$WORK/first.apk" "$WORK/second.apk" | annotate notice "Reproducible: two clean builds in different folders are the same APK"
    else
        { sha256sum "$WORK/first.apk" "$WORK/second.apk"; explain "$WORK/first.apk" "$WORK/second.apk"; } \
            | annotate error "Not reproducible: two clean builds in different folders differ"
        return 1
    fi
}

against() {
    local signed=${1:?the signed APK}
    mkdir -p "$WORK"
    step "A clean build elsewhere (as F-Droid would)" \
        build_in "$WORK/fdroid/build/io.github.c0decave.axrossamp" "$WORK/fdroid/cargo" "$WORK/rebuilt.apk"
    python3 -m venv "$WORK/venv"
    "$WORK/venv/bin/pip" install -q --require-hashes -r scripts/ci/apksigcopier-requirements.txt
    if "$WORK/venv/bin/apksigcopier" compare "$signed" --unsigned "$WORK/rebuilt.apk"; then
        echo "$(basename "$signed") is the rebuilt APK, signed" | annotate notice "Reproducible: the signed APK is this commit's build"
    else
        { echo "apksigcopier compare: $(basename "$signed") is not the rebuilt APK with a signature";
          explain "$signed" "$WORK/rebuilt.apk"; } \
            | annotate error "Not reproducible: the signed APK differs from a rebuild"
        return 1
    fi
}

case "${1:-}" in
    twice) twice ;;
    against) against "${2:-}" ;;
    *) echo "usage: $0 twice | against SIGNED_APK" >&2; exit 2 ;;
esac
