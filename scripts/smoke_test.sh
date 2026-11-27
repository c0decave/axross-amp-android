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
#   - the music playing on in the background (60 s, then 300 s in all) and
#     with the screen off (the media session's state, the position going
#     on); the media keys in the background (pause, play, fast forward and
#     rewind by 15 s, play/pause, next, previous); a restart after a
#     force-stop and after the process was killed brings the list and the
#     track back without a pick ("restored:");
#   - the activity destroyed (started anew with CLEAR_TASK: the process
#     must end and start fresh) and, from Android 12 on, bold text switched
#     on and off (no new activity at all);
#   - a monkey: random touches and keys within the app, seed logged;
#   - all of it offline (airplane mode, no Wi-Fi, no mobile data), and the
#     app with no network permission, no socket, not in the inet group.
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
    found=$(adb logcat -d -b crash 2>/dev/null | grep -E -B2 -A8 "$PACKAGE" | head -n 30)
    if [ -n "$found" ]; then
        say "after '$step': the crash log says"
        printf '%s\n' "$found"
        return 1
    fi
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

# The app's media session as `dumpsys media_session` shows it: its
# PlaybackState line, its state (2 paused, 3 playing), its position (ms).
session_line() {
    adb shell dumpsys media_session 2>/dev/null | tr -d '\r' \
        | awk -v p="package=$PACKAGE" 'index($0, p) { f = 1 } f && /state=PlaybackState/ { print; exit }'
}
# (the state as a number - "state=3", or "state=PLAYING(3)" from Android 14)
session_state() { session_line | grep -oE '\{state=[^,]*' | grep -oE '[0-9]+' | tail -n 1; }
# (the position right after the state - not the "buffered position" after it)
session_position() { session_line | sed -n 's/.*{state=[^,]*, position=\([0-9]*\).*/\1/p'; }

# Waits until the session's state is $1, at most $2 seconds.
wait_state() {
    local want=$1 seconds=$2 waited=0
    while [ "$waited" -lt "$seconds" ]; do
        [ "$(session_state)" = "$want" ] && return 0
        sleep 1
        waited=$((waited + 1))
    done
    say "the session's state is $(session_state), not $want (after $seconds s): $(session_line)"
    return 1
}

# The seconds into the track the app said last while playing (the debug
# APK logs it every 15 s and at every change).
played() {
    app_log | grep "playing: " | tail -n 1 | sed -n 's/.* playing: \([0-9]*\) s into.*/\1/p'
}

# A media key (the app in the background: it goes to the media session).
media_key() {
    adb shell input keyevent "$1"
    sleep 2
}

# Airplane mode, no Wi-Fi, no mobile data: everything after runs offline.
go_offline() {
    if ! adb shell cmd connectivity airplane-mode enable >/dev/null 2>&1; then
        adb shell settings put global airplane_mode_on 1
        adb shell am broadcast -a android.intent.action.AIRPLANE_MODE --ez state true >/dev/null 2>&1
    fi
    adb shell svc wifi disable >/dev/null 2>&1
    adb shell svc data disable >/dev/null 2>&1
    say "offline: airplane mode $(adb shell settings get global airplane_mode_on | tr -d '\r'), no Wi-Fi, no mobile data"
}

# The app cannot reach the network and did not try: its process is not in
# the inet group (no INTERNET permission: the kernel lets it open no
# network socket), and no socket of its uid is in /proc/net.
no_network() {
    local pid status groups uid sockets
    pid=$(app_pid)
    [ -n "$pid" ] || { say "no network: no process to look at"; return 0; }
    status=$(adb shell cat "/proc/$pid/status" | tr -d '\r')
    groups=$(printf '%s\n' "$status" | sed -n 's/^Groups:[[:space:]]*//p')
    uid=$(printf '%s\n' "$status" | awk '/^Uid:/ { print $2 }')
    case " $groups " in
        *" 3003 "*) say "the app's process is in the inet group: it may open sockets"; return 1 ;;
    esac
    sockets=$(adb shell cat /proc/net/tcp /proc/net/tcp6 /proc/net/udp /proc/net/udp6 2>/dev/null \
        | tr -d '\r' | awk -v u="$uid" '$8 == u' | wc -l)
    [ "${sockets:-0}" -eq 0 ] || { say "the app (uid $uid) has $sockets network sockets"; return 1; }
    say "no network: uid $uid not in the inet group (groups: ${groups:-none}), no network socket"
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
    # (no spaces in the names: the device's shell splits am's arguments;
    # first a track long enough to play on in the background)
    python3 - "$OUT/long.wav" <<'PY'
import math, struct, sys, wave
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(8000)
    w.writeframes(b"".join(struct.pack("<h", int(6000 * math.sin(2 * math.pi * 330 * i / 8000)))
                           for i in range(8000 * 420)))
PY
    adb push "$OUT/long.wav" "$dir/0-long.wav" >/dev/null
    rm -f "$OUT/long.wav"
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
    local paths all
    all=$(adb shell ls "$dir" | tr -d '\r' | sed "s#^#$dir/#")
    # (first the short tracks and the one being copied in: it is passed over)
    paths=$(printf '%s\n' "$all" | grep -v "0-long" | paste -sd, -)
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
    # (then the long track first, for the background: the others behind it)
    paths=$(printf '%s\n' "$all" | grep -v "growing" | paste -sd, -)
    say "handing over: $paths"
    start_activity --esa amp.paths "$paths" || return 1
    wait_for_line "opening 0-long.wav (to play" 20 || { say "the long track was not opened"; return 1; }
    sleep 5
    [ "$(app_pid)" = "$pid" ] || { say "the process died while playing"; return 1; }
    healthy "the music" || return 1
    wait_state 3 20 || { say "not playing"; return 1; }

    say "playing on in the background: home, then 60 s"
    adb shell input keyevent KEYCODE_HOME
    sleep 60
    [ "$(app_pid)" = "$pid" ] || { say "the process is gone in the background"; return 1; }
    wait_state 3 5 || { say "not playing after 60 s in the background"; return 1; }
    local p1 p2 p3
    p1=$(played)
    say "60 s in the background: playing, ${p1:-?} s into the track"
    [ "${p1:-0}" -ge 45 ] || { say "the position did not go on in the background"; return 1; }
    healthy "60 s in the background" || return 1

    say "the screen off, 20 s"
    adb shell input keyevent KEYCODE_SLEEP
    sleep 20
    [ "$(app_pid)" = "$pid" ] || { say "the process is gone with the screen off"; return 1; }
    wait_state 3 5 || { say "not playing with the screen off"; return 1; }
    p2=$(played)
    say "the screen off: playing, ${p2:-?} s into the track"
    [ "${p2:-0}" -gt "${p1:-0}" ] || { say "the position did not go on with the screen off"; return 1; }
    adb shell input keyevent KEYCODE_WAKEUP
    sleep 1
    adb shell input keyevent 82
    healthy "the screen off" || return 1

    say "media keys, the app in the background"
    adb shell input keyevent KEYCODE_HOME
    sleep 2
    media_key KEYCODE_MEDIA_PAUSE
    wait_state 2 5 || { say "the pause key did not pause"; return 1; }
    media_key KEYCODE_MEDIA_PLAY
    wait_state 3 5 || { say "the play key did not play"; return 1; }
    p1=$(session_position)
    media_key KEYCODE_MEDIA_FAST_FORWARD
    p2=$(session_position)
    say "fast forward: ${p1:-?} ms -> ${p2:-?} ms"
    [ "${p2:-0}" -ge $(( ${p1:-0} + 10000 )) ] || { say "fast forward did not go on 15 s"; app_log | grep "media:" | tail -n 3; return 1; }
    media_key KEYCODE_MEDIA_REWIND
    p3=$(session_position)
    say "rewind: ${p2:-?} ms -> ${p3:-?} ms"
    [ "${p3:-0}" -le $(( ${p2:-0} - 10000 )) ] || { say "rewind did not go back 15 s"; return 1; }
    media_key KEYCODE_MEDIA_PLAY_PAUSE
    wait_state 2 5 || { say "play/pause did not pause"; return 1; }
    media_key KEYCODE_MEDIA_PLAY_PAUSE
    wait_state 3 5 || { say "play/pause did not play"; return 1; }
    media_key KEYCODE_MEDIA_NEXT
    wait_for_line "media: next" 5 || { say "next did not arrive"; return 1; }
    media_key KEYCODE_MEDIA_PREVIOUS
    wait_for_line "media: previous" 5 || { say "previous did not arrive"; return 1; }
    app_log | grep -E "media: |opening .* to play" | tail -n 12
    [ "$(app_pid)" = "$pid" ] || { say "the process died on a media key"; return 1; }
    healthy "the media keys" || return 1
    no_network || return 1

    say "the system's folder picker opened for 10 s, then cancelled: the music plays on"
    start_activity || return 1
    sleep 2
    media_key KEYCODE_MEDIA_PLAY
    wait_state 3 5 || return 1
    local before after
    before=$(played)
    adb shell am start -a android.intent.action.OPEN_DOCUMENT_TREE >/dev/null 2>&1
    sleep 10
    [ "$(session_state)" = 3 ] || { say "the music stopped while the picker was open"; return 1; }
    adb shell input keyevent KEYCODE_BACK
    sleep 2
    start_activity || return 1
    sleep 14
    after=$(played)
    say "the picker: ${before:-?} s -> ${after:-?} s, the session's state $(session_state)"
    [ "$(session_state)" = 3 ] || { say "not playing after the picker was cancelled"; return 1; }
    [ "${after:-0}" -ge $(( ${before:-0} + 10 )) ] || { say "the position did not go on while the picker was open"; return 1; }
    healthy "the picker cancelled" || return 1

    say "a new choice that cannot be played: the old track plays on"
    local title
    title=$(app_log | grep "playing: " | tail -n 1 | sed -n 's/.* s into //p')
    start_activity --esa amp.paths "$dir/no-such-file.mp3" || return 1
    wait_for_line "could not be played\|cannot be opened" 15 || true
    sleep 3
    [ "$(session_state)" = 3 ] || { say "the music stopped for a choice that could not be played"; return 1; }
    say "still playing: $(app_log | grep "playing: " | tail -n 1 | sed -n 's/.* s into //p') (was $title)"
    healthy "a choice that could not be played" || return 1

    say "a new choice: the old track plays until the new one starts"
    start_activity --esa amp.paths "$dir/1-sine.mp3,$dir/3-sine.flac" || return 1
    wait_for_line "opening 1-sine.mp3 (to play" 15 || { say "the new choice was not opened"; return 1; }
    sleep 2
    [ -n "$(app_pid)" ] || { say "the process died on a new choice"; return 1; }
    healthy "a new choice" || return 1
    # (the long track again for the rest)
    start_activity --esa amp.paths "$dir/0-long.wav" || return 1
    wait_for_line "opening 0-long.wav (to play" 15 || true
    adb shell input keyevent KEYCODE_HOME
    sleep 1

    say "playing on in the background, 300 s in all"
    media_key KEYCODE_MEDIA_PLAY
    sleep 220
    [ "$(app_pid)" = "$pid" ] || { say "the process is gone after 300 s in the background"; return 1; }
    say "after 300 s: the session's state $(session_state)"
    healthy "300 s in the background" || return 1
    kill "$copier" 2>/dev/null
    wait "$copier" 2>/dev/null
    start_activity || return 1
    sleep 3
    healthy "back after 300 s" || return 1

    say "restart: the state comes back without a pick"
    media_key KEYCODE_MEDIA_PAUSE
    sleep 2
    adb shell am force-stop "$PACKAGE"
    sleep 2
    adb logcat -c
    start_activity || return 1
    if ! wait_for_line "restored:" 30; then
        say "nothing restored after a force-stop"
        return 1
    fi
    app_log | grep "restored:" | tail -n 1
    pid=$(app_pid)
    say "restart after the process was killed"
    adb shell run-as "$PACKAGE" kill -9 "$pid" 2>/dev/null || adb shell am force-stop "$PACKAGE"
    sleep 3
    adb logcat -c
    start_activity || return 1
    if ! wait_for_line "restored:" 30; then
        say "nothing restored after the process was killed"
        return 1
    fi
    app_log | grep "restored:" | tail -n 1
    pid=$(app_pid)
    healthy "the restarts" || return 1

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

    no_network || return 1

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
# (an install takes longer than other calls on an emulator just booted:
# its own time limit, and once more if the first did not end)
installed=0
for try in 1 2; do
    if timeout --kill-after=10 300 "$ADB_BIN" install -r "$APK"; then
        installed=1
        break
    fi
    say "the install did not end (try $try)"
    sleep 20
done
[ "$installed" = 1 ] || { say "the APK could not be installed"; exit 1; }
# (Android 13+: the notification permission granted beforehand - its
# question would come up over the app at the first play)
if [ "${API:-0}" -ge 33 ]; then
    adb shell pm grant "$PACKAGE" android.permission.POST_NOTIFICATIONS
fi
if adb shell dumpsys package "$PACKAGE" | tr -d '\r' | grep -q "android.permission.INTERNET"; then
    say "the app holds a network permission"
    exit 1
fi
go_offline

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
