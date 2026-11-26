#!/usr/bin/env bash
# The emulator's smoke test in CI (ci.yml, job smoke):
#
#   scripts/ci/emulator.sh settings API   the emulator for that API level, as
#                                         step outputs for the emulator action
#                                         (arch, target, cores, ram-size,
#                                         disk-size, boot-timeout, build,
#                                         options)
#   scripts/ci/emulator.sh smoke APK OUT  scripts/smoke_test.sh, with what it
#                                         found as annotations of the job
#                                         (the renderer, the first frame, and
#                                         on a failure the log's telling lines)
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
. scripts/ci/lib.sh

settings() {
    local api=${1:?the API level}
    local arch=x86_64 target=default cores=3 ram=3072M disk=6000M timeout=900 build=""
    # SwiftShader: the emulator's GPU in software, OpenGL ES 3 and Vulkan
    # (where the image offers it), the same on every runner. With its sound
    # device (no -noaudio): the smoke test plays music.
    local options="-no-window -gpu swiftshader_indirect -no-boot-anim -no-snapshot -camera-back none -camera-front none"
    case "$api" in
        26) ram=2048M ;;
    esac
    {
        echo "arch=$arch"
        echo "target=$target"
        echo "cores=$cores"
        echo "ram-size=$ram"
        echo "disk-size=$disk"
        echo "boot-timeout=$timeout"
        echo "build=$build"
        echo "options=$options"
    } | tee -a "${GITHUB_OUTPUT:-/dev/null}"
}

smoke() {
    local apk=${1:?the APK} out=${2:-smoke-output} rc=0 api
    api=$(timeout 60 adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)
    # (a watchdog under the job's own limit: what was found is still told)
    timeout --kill-after=30 1500 bash scripts/smoke_test.sh "$apk" "$out" 2>&1 \
        | tee "$CI_LOGS/smoke.log" || rc=$?
    [ "$rc" -ne 124 ] || echo "smoke: FAIL - the smoke test did not end within 25 minutes" | tee -a "$CI_LOGS/smoke.log"
    set +e
    {
        echo "API $api, $(timeout 60 adb shell getprop ro.product.cpu.abi 2>/dev/null | tr -d '\r')"
        grep -E 'smoke: PASS|smoke: FAIL|::warning' "$CI_LOGS/smoke.log" | sed 's/^::warning[^:]*:://'
        grep -h -E 'starting|backend:|renderer:|first frame|frame after resume|panic' "$out/app-log.txt" 2>/dev/null | head -n 20
    } | annotate "$([ "$rc" -eq 0 ] && echo notice || echo error)" "Smoke test on API $api"
    if [ "$rc" -ne 0 ]; then
        tail -n 70 "$out/app-log.txt" 2>/dev/null | annotate error "The app's own log (API $api)"
        cat "$out"/logcat*.txt 2>/dev/null \
            | grep -E 'AndroidRuntime|FATAL|DEBUG +:|libc +:|linker|UnsatisfiedLink|dlopen|NativeActivity|axross|wgpu|RustPanic|Vulkan|EGL|ActivityManager: (Start proc|Process|Force)|ActivityTaskManager: (START|Displayed)' \
            | head -n 90 | annotate error "logcat, the telling lines (API $api)"
        tail -n 40 "$CI_LOGS/smoke.log" | annotate error "The smoke test's own output (API $api)"
    fi
    set -e
    return "$rc"
}

case "${1:-}" in
    settings) shift; settings "$@" ;;
    smoke) shift; smoke "$@" ;;
    *) echo "usage: $0 settings API | smoke APK [OUT]" >&2; exit 2 ;;
esac
