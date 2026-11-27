#!/usr/bin/env bash
# The README's screenshots, from an emulator (screenshots.yml):
#
#   scripts/ci/screenshots.sh APK OUT_DIR
#
# The demo music (scripts/ci/demo_audio.py: made up here, nobody's music,
# nothing personal on the screen) is handed over as if picked (the debug
# APK's amp.paths extra) and the app is photographed at defined steps:
# the player with its controls and the cover, the faces, the list of
# faces, the media notification, the lock screen, and on its side.
set -u

APK=${1:?the APK}
OUT=${2:-shots}
PACKAGE=io.github.c0decave.axrossamp.debug
ACTIVITY=io.github.c0decave.axrossamp.MainActivity
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ADB_BIN=$(command -v adb) || { echo "no adb"; exit 1; }
adb() { timeout --kill-after=10 60 "$ADB_BIN" "$@"; }
say() { printf 'shots: %s\n' "$*"; }
shot() {
    # (a system dialog in the way - the emulator's launcher "isn't
    # responding" while the emulator is busy - closed first)
    adb shell am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS >/dev/null 2>&1
    sleep 1
    adb exec-out screencap -p > "$OUT/$1.png" 2>/dev/null
    say "$1.png ($(stat -c %s "$OUT/$1.png" 2>/dev/null || echo 0) bytes)"
}
size() { adb shell wm size | tr -d '\r' | sed -n 's/.*: \([0-9]*\)x\([0-9]*\)$/\1 \2/p' | tail -1; }
density() { adb shell wm density | tr -d '\r' | sed -n 's/.*: \([0-9]*\)$/\1/p' | tail -1; }

mkdir -p "$OUT"
timeout 300 "$ADB_BIN" wait-for-device
# (the emulator's own apps settle after the boot first)
sleep 45
timeout --kill-after=10 300 "$ADB_BIN" install -r "$APK" || timeout --kill-after=10 300 "$ADB_BIN" install -r "$APK" || exit 1
API=$(adb shell getprop ro.build.version.sdk | tr -d '\r')
[ "${API:-0}" -ge 33 ] && adb shell pm grant "$PACKAGE" android.permission.POST_NOTIFICATIONS
adb shell settings put system accelerometer_rotation 0
adb shell settings put system user_rotation 0
# (a clean status bar: the clock fixed, no notifications but the app's)
adb shell settings put global sysui_demo_allowed 1
adb shell am broadcast -a com.android.systemui.demo -e command enter >/dev/null
adb shell am broadcast -a com.android.systemui.demo -e command clock -e hhmm 1200 >/dev/null
adb shell am broadcast -a com.android.systemui.demo -e command notifications -e visible false >/dev/null

say "the demo music"
python3 -I "$ROOT/scripts/ci/demo_audio.py" "$OUT/music"
# (the app started once first: its folder on the shared storage is made
# for it then - one made by the shell before it is not readable by the app)
adb shell am start -W -n "$PACKAGE/$ACTIVITY" >/dev/null
sleep 5
adb shell am force-stop "$PACKAGE"
dir="/sdcard/Android/data/$PACKAGE/files/Demo_Tones"
adb shell mkdir -p "$dir"
paths=""
for f in "$OUT"/music/*.flac; do
    name=$(basename "$f" | tr ' ' '_')
    adb push "$f" "$dir/$name" >/dev/null
    paths="$paths${paths:+,}$dir/$name"
done
adb shell "touch -t 202001010000 $dir/*"
rm -rf "$OUT/music"

read -r W H <<< "$(size)"
DPI=$(density)
# (points to pixels: the screen's layout in dp)
px() { echo $(( $1 * DPI / 160 )); }
say "screen ${W}x${H} at $DPI dpi, API $API"

adb shell am start -W -n "$PACKAGE/$ACTIVITY" --esa amp.paths "$paths" >/dev/null
sleep 4
shot 01-player-controls
sleep 9
shot 02-face-spectrum
# (a swipe to the left: the next face, in the order of the list of faces)
swipe() { adb shell input swipe $((W * 3 / 4)) $((H / 2)) $((W / 4)) $((H / 2)) 200; sleep "${1:-2}"; }
for i in 3 4 5 6 7 8 9 10; do
    swipe 4
    shot "$(printf '%02d' "$i")-face"
done
# (further faces by their keys, as the app's log names them: a swipe
# the emulator was too busy for is tried again)
to_face() {
    local tries=0
    while [ "$(adb logcat -d -s axrossamp:V | tr -d '\r' | sed -n 's/.*axrossamp: face: //p' | tail -n 1)" != "$1" ]; do
        [ "$tries" -lt 60 ] || { say "face $1 not reached"; return 1; }
        swipe 3
        tries=$((tries + 1))
    done
    sleep 4
}
to_face cover
shot 17-face-cover
adb shell input tap $((W / 2)) $((H / 2))
sleep 1
shot 18-cover-controls
sleep 9
to_face synthwave
shot 19-face-synthwave
to_face galaxy
shot 20-face-galaxy
to_face skyline
shot 21-face-skyline
say "the list of faces"
adb shell input tap $((W / 2)) $((H / 2))
sleep 1
# (the "Faces" button: the top bar's first, under the status bar)
adb shell input tap "$(px 60)" "$(px 80)"
sleep 2
shot 11-faces-list
adb shell input keyevent KEYCODE_BACK
sleep 1
say "the media notification"
adb shell cmd statusbar expand-notifications
sleep 3
# (no shot(): closing the system dialogs would close the shade too)
adb exec-out screencap -p > "$OUT/13-notification.png" 2>/dev/null
say "13-notification.png"
adb shell cmd statusbar collapse
sleep 1
say "the lock screen (a PIN set for it: the emulator has no lock screen)"
adb shell locksettings set-pin 2580 >/dev/null 2>&1
adb shell input keyevent KEYCODE_SLEEP
sleep 3
adb shell input keyevent KEYCODE_WAKEUP
sleep 3
adb exec-out screencap -p > "$OUT/14-lock-screen.png" 2>/dev/null
say "14-lock-screen.png"
adb shell input keyevent 82
sleep 2
adb shell input text 2580
adb shell input keyevent 66
sleep 2
adb shell locksettings clear --old 2580 >/dev/null 2>&1
adb shell wm dismiss-keyguard >/dev/null 2>&1
sleep 2
say "on its side"
# (the list from the start again: playing, not at its end)
adb shell am start -W -n "$PACKAGE/$ACTIVITY" --esa amp.paths "$paths" >/dev/null
sleep 2
adb shell settings put system user_rotation 1
adb shell wm user-rotation lock 1 >/dev/null 2>&1
sleep 8
say "rotation: $(adb shell dumpsys window displays | tr -d '\r' | grep -o 'mCurrentRotation=[^ ]*' | head -1)"
adb shell input tap $((H / 2)) $((W / 2))
sleep 1
shot 15-landscape-controls
sleep 9
shot 16-landscape-face
adb shell wm user-rotation lock 0 >/dev/null 2>&1
adb shell settings put system user_rotation 0
adb shell am broadcast -a com.android.systemui.demo -e command exit >/dev/null
adb logcat -d -s axrossamp:V > "$OUT/app-log.txt"
ls -l "$OUT"
