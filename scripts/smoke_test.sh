#!/usr/bin/env bash
# The emulator's smoke test (CI; any device or emulator `adb` sees will do):
#   scripts/smoke_test.sh path/to/app-debug.apk [output-dir]
#
# Installs the APK, starts the activity and waits for the line the app
# writes when its first frame is drawn ("first frame", tag axrossamp). Then
# the app must stay alive and answering through:
#
#   - touches, a rotation, a trip to the home screen and back (the window's
#     surface lost and made anew: "frame after resume");
#   - a rotation loop and a pause/resume loop;
#   - music played - files handed over as if picked (the debug APK's
#     amp.paths extra), one of them still being copied in while it is
#     picked: it must be passed over ("still being copied"), the others
#     opened ("done in"), and nothing may block the main thread;
#   - the activity destroyed (started anew with CLEAR_TASK: the process
#     must end and start fresh) and, from Android 12 on, bold text switched
#     on and off (no new activity at all);
#   - a monkey: random touches and keys within the app, seed logged.
#
# After every step: no ANR, no crash, no panic, no blocked main thread in
# the log, the process not "not responding".
#
# With the renderer the app chooses by itself first (Vulkan, else OpenGL
# ES); if that does not get to a first frame, once more with OpenGL ES
# forced - an emulator's Vulkan is not a phone's. Which one worked is said,
# and a fallback is a warning in the job, not a silent pass.
#
# Leaves logcat.txt, app-log.txt and screen-*.png in the output directory.
set -u

APK=${1:?the APK to test}
OUT=${2:-smoke-output}
PACKAGE=${AMP_PACKAGE:-io.github.c0decave.axrossamp.debug}
ACTIVITY=io.github.c0decave.axrossamp.MainActivity
FIRST_FRAME_SECONDS=${AMP_FIRST_FRAME_SECONDS:-120}
ALIVE_SECONDS=${AMP_ALIVE_SECONDS:-15}
LOOPS=${AMP_LOOPS:-8}
MONKEY_EVENTS=${AMP_MONKEY_EVENTS:-1500}
SEED=${AMP_SEED:-$(( $(date +%s) % 100000 ))}
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

mkdir -p "$OUT"

# Every adb call has a time limit: an emulator whose GPU translation hangs
# leaves adb waiting for ever, and the job would end at its own time limit
# with nothing said.
ADB_TIMEOUT=${AMP_ADB_TIMEOUT:-60}
ADB_BIN=$(command -v adb) || { echo "smoke: no adb on PATH"; exit 1; }
adb() { timeout --kill-after=10 "$ADB_TIMEOUT" "$ADB_BIN" "$@"; }

say() { printf '%s\n' "smoke: $*"; }

app_log() { adb logcat -d -s axrossamp:V 2>/dev/null; }

app_pid() { adb shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r' | awk '{print $1}'; }

API=$(adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r')

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

# No ANR, crash, panic or blocked main thread so far ($1: the step).
healthy() {
    local step=$1 found
    found=$(adb logcat -d 2>/dev/null | grep -E -A3 \
        "ANR in $PACKAGE|Input dispatching timed out|FATAL EXCEPTION|Fatal signal|axrossamp.*panic:|main thread has been blocked" \
        | grep -E -B1 -A3 "$PACKAGE|axrossamp|Input dispatching" | head -n 30)
    if [ -n "$found" ]; then
        say "after '$step': the log says"
        printf '%s\n' "$found"
        return 1
    fi
    if adb shell dumpsys activity processes 2>/dev/null | grep -A20 "$PACKAGE" | grep -q "notResponding=true"; then
        say "after '$step': the process is not responding"
        return 1
    fi
    return 0
}

start_activity() {
    adb shell am start -W -n "$PACKAGE/$ACTIVITY" "$@" >/dev/null || return 1
}

# The activity shown again after the home screen, a frame drawn.
home_and_back() {
    local before
    before=$(app_log | grep -c -- "frame after resume")
    adb shell input keyevent KEYCODE_HOME
    sleep "$1"
    start_activity || return 1
    local waited=0
    while [ "$(app_log | grep -c -- "frame after resume")" -le "$before" ]; do
        [ "$waited" -lt 60 ] || { say "no frame after the activity was shown again"; return 1; }
        sleep 1
        waited=$((waited + 1))
    done
}

# One run with the renderer $1: auto (the app chooses), gl or vulkan.
run() {
    local backend=$1 pid
    adb shell am force-stop "$PACKAGE" >/dev/null 2>&1
    adb shell setprop debug.axrossamp.backend "$backend"
    adb logcat -c
    say "starting the activity (renderer: $backend)"
    start_activity || return 1

    if ! wait_for_line "first frame" "$FIRST_FRAME_SECONDS"; then
        say "no first frame within $FIRST_FRAME_SECONDS s"
        return 1
    fi
    pid=$(app_pid)
    if [ -z "$pid" ]; then
        say "the process is gone right after its first frame"
        return 1
    fi
    app_log | grep -- "backend:" | tail -1
    app_log | grep -- "renderer:" | tail -1
    app_log | grep -- "first frame" | tail -1
    screenshot "$backend-first"

    say "alive for $ALIVE_SECONDS s?"
    sleep "$ALIVE_SECONDS"
    [ "$(app_pid)" = "$pid" ] || { say "the process died or was started again"; return 1; }
    healthy "the start" || return 1

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
    home_and_back 4 || return 1
    sleep 3
    [ "$(app_pid)" = "$pid" ] || { say "the process died after it was shown again"; return 1; }
    screenshot "$backend-resumed"
    healthy "home and back" || return 1
    return 0
}

# The loops, the music, the destroyed activity, the monkey: with the
# renderer that passed.
stress() {
    local pid
    pid=$(app_pid)

    say "rotation loop ($LOOPS times)"
    for i in $(seq 1 "$LOOPS"); do
        adb shell settings put system user_rotation $((i % 4))
        sleep 0.$((RANDOM % 9 + 5))
    done
    adb shell settings put system user_rotation 0
    sleep 3
    [ "$(app_pid)" = "$pid" ] || { say "the process died in the rotation loop"; return 1; }
    healthy "the rotation loop" || return 1

    say "pause/resume loop ($LOOPS times, the last one long enough to rest)"
    for i in $(seq 1 "$LOOPS"); do
        home_and_back $((RANDOM % 3 + 1)) || return 1
    done
    home_and_back 8 || return 1
    [ "$(app_pid)" = "$pid" ] || { say "the process died in the pause/resume loop"; return 1; }
    healthy "the pause/resume loop" || return 1

    say "music, one of the files still being copied in"
    local dir="/sdcard/Android/data/$PACKAGE/files/music" copier
    adb shell mkdir -p "$dir"
    # (no spaces in the names: the device's shell splits am's arguments)
    adb push "$ROOT/tests/media/sine.mp3" "$dir/1-sine.mp3" >/dev/null
    adb push "$ROOT/tests/media/sine.flac" "$dir/3-sine.flac" >/dev/null
    for f in vbr-noxing-long.mp3 chirp.opus; do
        [ -f "$ROOT/axross-amp/tests/media/$f" ] && adb push "$ROOT/axross-amp/tests/media/$f" "$dir/4-$f" >/dev/null
    done
    # (copied a while ago, as far as their times say)
    adb shell "touch -t 202001010000 $dir/*"
    adb push "$ROOT/tests/media/sine.mp3" "/data/local/tmp/amp-source.mp3" >/dev/null
    # (a copy that goes on for half a minute: a piece a second)
    adb shell "rm -f $dir/2-growing.mp3; for i in \$(seq 1 30); do cat /data/local/tmp/amp-source.mp3 >> $dir/2-growing.mp3; sleep 1; done" &
    copier=$!
    sleep 2
    local paths
    paths=$(adb shell ls "$dir" | tr -d '\r' | sed "s#^#$dir/#" | paste -sd, -)
    say "handing over: $paths"
    adb logcat -c
    start_activity --esa amp.paths "$paths" || return 1
    if ! wait_for_line "done in" 20; then
        say "no track opened within 20 s"
        return 1
    fi
    if ! wait_for_line "still being copied" 30; then
        say "the file being copied was not passed over"
        return 1
    fi
    app_log | grep -E "test tracks|done in|still being copied|tracks in the list|output:" | head -n 12
    sleep 5
    [ "$(app_pid)" = "$pid" ] || { say "the process died while playing"; return 1; }
    healthy "the music" || return 1
    say "home while it plays (it pauses; after the rest, a new engine)"
    home_and_back 8 || return 1
    kill "$copier" 2>/dev/null
    wait "$copier" 2>/dev/null
    healthy "home while playing" || return 1

    if [ "${API:-0}" -ge 31 ]; then
        say "bold text on and off (no new activity: the same process)"
        adb shell settings put secure font_weight_adjustment 300
        sleep 3
        adb shell settings put secure font_weight_adjustment 0
        sleep 3
        [ "$(app_pid)" = "$pid" ] || { say "the process changed on bold text"; return 1; }
        healthy "bold text" || return 1
    fi

    say "the activity destroyed (started anew with CLEAR_TASK): its process ends, a new one starts"
    local old=$pid waited=0
    adb logcat -c
    # (FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK: the running
    # activity is finished - onDestroy - and a new one started)
    adb shell am start -W -f 0x10008000 -n "$PACKAGE/$ACTIVITY" >/dev/null
    while [ "$(app_pid)" = "$old" ]; do
        [ "$waited" -lt 20 ] || { say "the process lives on after its activity was destroyed"; return 1; }
        sleep 1
        waited=$((waited + 1))
    done
    if ! wait_for_line "first frame" 60; then
        say "no first frame after the activity was destroyed"
        start_activity
        wait_for_line "first frame" 30 || return 1
        say "(a first frame only after one more start)"
    fi
    adb logcat -d -s axrossamp:V | grep -E "destroyed|starting" | tail -n 3
    pid=$(app_pid)
    healthy "the destroyed activity" || return 1

    say "monkey: $MONKEY_EVENTS events, seed $SEED"
    adb shell monkey -p "$PACKAGE" --pct-syskeys 0 --pct-appswitch 0 --throttle 50 -s "$SEED" "$MONKEY_EVENTS" > "$OUT/monkey.txt" 2>&1
    sleep 3
    [ -n "$(app_pid)" ] || { say "the process died under the monkey (seed $SEED)"; tail -n 20 "$OUT/monkey.txt"; return 1; }
    healthy "the monkey (seed $SEED)" || return 1
    return 0
}

timeout 300 "$ADB_BIN" wait-for-device || { say "no device"; exit 1; }
say "API $API"
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
if [ "$result" -eq 0 ]; then
    if stress; then
        say "PASS: the loops, the music, the destroyed activity, the monkey"
    else
        say "FAIL in the stress steps"
        result=1
    fi
fi

adb logcat -d > "$OUT/logcat.txt" 2>/dev/null
app_log > "$OUT/app-log.txt"
say "the app's own lines:"
tail -n 80 "$OUT/app-log.txt"
adb shell am force-stop "$PACKAGE" >/dev/null 2>&1
exit "$result"
