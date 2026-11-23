#!/usr/bin/env bash
# The emulator's smoke test (CI; any device or emulator `adb` sees will do):
#   scripts/smoke_test.sh path/to/app-debug.apk [output-dir]
#
# Installs the APK, starts the activity and waits for the line the app
# writes when its first frame is drawn ("first frame", tag axrossamp). Then
# the app must stay alive - the same process - through some seconds of
# drawing, a rotation, and a trip to the home screen and back (the window's
# surface is lost and made anew: "frame after resume").
#
# With the renderer the app chooses by itself first (Vulkan, else OpenGL
# ES); if that does not get to a first frame, once more with OpenGL ES
# forced - an emulator's Vulkan is not a phone's. Which one worked is said,
# and a fallback is a warning in the job, not a silent pass.
#
# Leaves logcat.txt and screen-*.png in the output directory.
set -u

APK=${1:?the APK to test}
OUT=${2:-smoke-output}
PACKAGE=${AMP_PACKAGE:-io.github.c0decave.axrossamp.debug}
ACTIVITY=io.github.c0decave.axrossamp.MainActivity
FIRST_FRAME_SECONDS=${AMP_FIRST_FRAME_SECONDS:-120}
ALIVE_SECONDS=${AMP_ALIVE_SECONDS:-15}

mkdir -p "$OUT"

say() { printf '%s\n' "smoke: $*"; }

app_log() { adb logcat -d -s axrossamp:V 2>/dev/null; }

app_pid() { adb shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r' | awk '{print $1}'; }

# Waits until the app's log has a line with $1, at most $2 seconds.
wait_for_line() {
    local needle=$1 seconds=$2 waited=0
    while [ "$waited" -lt "$seconds" ]; do
        if app_log | grep -q -- "$needle"; then
            return 0
        fi
        sleep 2
        waited=$((waited + 2))
    done
    return 1
}

screenshot() {
    adb exec-out screencap -p > "$OUT/screen-$1.png" 2>/dev/null || true
}

# One run with the renderer $1: auto (the app chooses), gl or vulkan.
run() {
    local backend=$1 pid
    adb shell am force-stop "$PACKAGE" >/dev/null 2>&1
    adb shell setprop debug.axrossamp.backend "$backend"
    adb logcat -c
    say "starting the activity (renderer: $backend)"
    adb shell am start -W -n "$PACKAGE/$ACTIVITY" || return 1

    if ! wait_for_line "first frame" "$FIRST_FRAME_SECONDS"; then
        say "no first frame within $FIRST_FRAME_SECONDS s"
        return 1
    fi
    pid=$(app_pid)
    if [ -z "$pid" ]; then
        say "the process is gone right after its first frame"
        return 1
    fi
    app_log | grep -- "renderer:" | tail -1
    app_log | grep -- "first frame" | tail -1
    screenshot "$backend-first"

    say "alive for $ALIVE_SECONDS s?"
    sleep "$ALIVE_SECONDS"
    [ "$(app_pid)" = "$pid" ] || { say "the process died or was started again"; return 1; }

    say "a tap (the controls hide), a swipe (the next face)"
    adb shell input tap 300 600
    adb shell input swipe 600 700 100 700 200
    sleep 3
    [ "$(app_pid)" = "$pid" ] || { say "the process died on a touch"; return 1; }
    screenshot "$backend-swiped"

    say "rotation to landscape and back"
    adb shell settings put system accelerometer_rotation 0
    adb shell settings put system user_rotation 1
    sleep 4
    [ "$(app_pid)" = "$pid" ] || { say "the process died on rotation"; return 1; }
    screenshot "$backend-landscape"
    adb shell settings put system user_rotation 0
    sleep 3

    say "to the home screen and back (the surface is lost and made anew)"
    adb shell input keyevent KEYCODE_HOME
    sleep 4
    [ "$(app_pid)" = "$pid" ] || { say "the process died when it was hidden"; return 1; }
    adb shell am start -W -n "$PACKAGE/$ACTIVITY" || return 1
    if ! wait_for_line "frame after resume" 60; then
        say "no frame after the activity was shown again"
        return 1
    fi
    sleep 3
    [ "$(app_pid)" = "$pid" ] || { say "the process died after it was shown again"; return 1; }
    screenshot "$backend-resumed"

    if app_log | grep -q -- "panic:"; then
        say "a panic was logged"
        return 1
    fi
    return 0
}

adb wait-for-device
say "installing $APK"
adb install -r "$APK" || { say "the APK could not be installed"; exit 1; }

result=1
if run auto; then
    say "PASS with the renderer the app chose"
    result=0
else
    adb logcat -d > "$OUT/logcat-auto.txt" 2>/dev/null
    say "retrying with OpenGL ES forced"
    if run gl; then
        echo "::warning title=smoke test::the app reached no first frame with its own choice of renderer on this emulator; it did with OpenGL ES forced (see logcat-auto.txt in the artifact)"
        say "PASS with OpenGL ES forced only"
        result=0
    else
        say "FAIL with either renderer"
    fi
fi

adb logcat -d > "$OUT/logcat.txt" 2>/dev/null
app_log > "$OUT/app-log.txt"
say "the app's own lines:"
cat "$OUT/app-log.txt"
adb shell am force-stop "$PACKAGE" >/dev/null 2>&1
exit "$result"
