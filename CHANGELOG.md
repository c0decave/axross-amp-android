# Changelog

## 0.1.1 - 2026-11-27

From the first phone's report on 0.1.0: the music stopped (and the app
seemed to crash) when it went to the background, and every start wanted
the files picked again.

- Music plays on in the background: with the app hidden, the screen off,
  on the lock screen. The player runs on a thread of its own for as long
  as the process lives (it no longer depends on the window, which Android
  takes away when the app is hidden), and a foreground service of the type
  mediaPlayback keeps the process while music plays.
- A media session with a media notification: play / pause, previous /
  next, 15 s back / on and the seek bar - in the notification, on the lock
  screen, in Android 13's media controls, from headsets and Bluetooth.
  Previous starts the track again when it is more than 3 s in. Android 13+
  asks for the notification permission at the first play. Media keys work
  with the app on screen too (winit kept them from the session before).
- What was picked comes back at the next start: the list, the track and
  where it was, paused (the pickers' read permission is kept; a document
  that is gone or no longer readable is said and passed over).
- Hidden, only the picture stops: the meters rest after 5 s; paused and
  hidden for a minute the sound output is closed until the next play.
- No network at all: the app asks for no network permission and has no
  network code - the desktop crate leaves its internet radio and its
  update out of the Android build (no HTTP client, no TLS). Everything
  works in airplane mode. CI fails on a network crate in the Android
  build or a network permission in the APK, and runs the emulator's tests
  offline with the app's process outside Android's network group.
- The music plays on while a picker is open; a new pick changes the track
  only when the new one is open, and one that cannot be played leaves the
  old track playing, said.
- How the app ended last time (Android 11+) is logged, and a crash or an
  ANR is said on the screen at the next start - for the next report.
- Supported: Android 9 (API 28) to 16 (API 36), tested in CI's emulators
  of API 28, 34 and 36 (and run on 26, which still installs); the smoke
  test plays on in the background (60 s and 300 s) and with the screen
  off, checks every media key against the media session and the position,
  and restarts after a force-stop and a kill.

## 0.1.0 - 2026-11-26

The first build: a proof that axross-amp's player and its visualizations run
as an Android app. Step 1 of two (see docs/ARCHITECTURE.md).

- The desktop program's library crate (engine, analysis, the 69 faces, the
  flash guard with its limits unchanged) built for Android, used through a
  git submodule - not copied.
- One touch screen for portrait and landscape: a face over the whole
  display; swipe left / right for the next / previous face, tap to show or
  hide the controls; title, seek bar, previous / play-pause / next; a list
  of the faces; touch targets of at least 48 dp; safe insets and display
  cutouts respected; the screen kept on while music plays.
- Music from a folder (with its subfolders) or from files, through the
  Storage Access Framework's pickers; no permission asked for. content://
  documents are opened by descriptor and played in place, or copied to the
  cache where a provider does not allow that.
- wgpu on Vulkan (Android 10 and later with a GPU's Vulkan driver) or
  OpenGL ES 3 (older Android, the emulator) - one of them, chosen at the
  start (`debug.axrossamp.backend` forces one); cpal on AAudio; symphonia
  with libopus.
- Lifecycle: the surface given up and made anew when the app is hidden and
  shown; the music pauses when the app is hidden (no background playback
  yet), on the loss of audio focus and when headphones are pulled out.
- CI: the supply-chain gate, the host's unit tests, debug APKs for
  arm64-v8a and x86_64, a smoke test in an emulator. A release workflow
  that signs with a key from the repository's secrets and drafts a release.

Since the first preview (the debug APK preview-0.1.0-1), after the first
reports from a phone and an audit of crashes and hangs:

- The app no longer hangs when music starts to play. Keeping the screen
  on through android-activity's window flags deadlocked the window's
  thread and Android's main thread as soon as the activity paused (the
  first track played on, nothing else answered): the screen is kept on by
  the Java side now.
- A folder picked while music is still being copied into it no longer
  holds the app up. The folder's tracks come in batches while it is read
  (the first plays at once, the list fills behind it; a newer pick ends
  the reading of an older one); every document is opened off the
  window's thread with a time limit, and a newer request sets aside an
  opening the system does not answer; a track still being written (empty,
  changed in the last 5 s, or growing between two looks) is passed over
  and tried again later; a copy where a document cannot be opened in
  place stops at 1 GiB or 60 s.
- Seeking: 15 s back and on beside play; a long press on previous or next
  winds back or on, faster the longer it is held; a double tap on the
  picture's left or right third skips 10 s (more taps skip on), with a
  hint on the picture; the seek bar is heard while it is dragged, a tap on
  it jumps there; a seek asked for while a track is still opening is done
  once it is open; past the end a skip goes on to the next track.
- Step logs and watchdogs: each step of a pick and of opening a track is
  logged with its time (file names only in debug builds); a step taking
  longer than 2 s is logged while it waits, and a main thread blocked for
  2 s logs the Java stacks and the Rust side's steps.
- Stability (an audit of crashes, hangs and races):
  - the activity destroyed - the task swiped away, bold text switched on,
    "Don't keep activities" - no longer freezes the app: the process ends
    with it (and every configuration change Android 16 knows is taken
    without a new activity); a second activity in the same process (a new
    task started over the old one) starts afresh in a new process instead
    of a second android_main that cannot draw;
  - a panic no longer ends the whole app: the library unwinds (a decoder
    thread that panics on an odd file skips that track; the window's
    thread ends the process cleanly instead of leaving music without a
    window); the debug APK is built with the release's settings (no
    overflow checks or debug assertions);
  - wgpu's errors are logged instead of ending the app; a lost graphics
    device moves the faces to the CPU, and where Vulkan failed (the window
    did not come up, the device was lost) the next start draws with
    OpenGL ES;
  - the GPU's measurement at the start runs on a thread of its own with a
    time limit (a stalling driver no longer blocks the start), and not at
    all on a software rasterizer; the audio focus and the window's flags
    are set off the window's thread;
  - hidden for 5 s, the app rests: the engine and its output stream end,
    the meters stop, the watchdogs sleep - no wakelock, no drained
    battery; shown again, a new engine opens the track where it was;
  - no sound output (a Bluetooth device that keeps dropping) is retried by
    itself every 5 to 30 s instead of for good;
  - tracks still being copied: a pause or the app hidden is no longer
    overridden by the retry, only one track is tried a round, with growing
    waits, and a copy that does not go on is given up; an empty file is a
    failure;
  - the track playing and the one queued are never let go of by the
    opener (a seek or a new output read them again); copies go to the
    cache (the system can reclaim it), 512 MiB at most; no more than 4
    documents wait on the system at once, and the Java side cancels an
    opening after 11 s; a file smaller than its provider says is still
    being copied;
  - a track that ended as a pause came does not start the next; a wind
    asked for while a track opens stays a wind.
- Tests: the decoders on files cut short and damaged (the audit's fuzz, as
  a test), the copying retries, the opener's limits; the emulator's smoke
  test now plays music with a file still being copied in, loops rotations
  and pauses, destroys the activity, runs a monkey, and fails on any ANR,
  crash or blocked main thread; the APK is checked for 16 KB alignment.
- New launcher icon: the Synthwave Sun, with a themed-icon layer for
  Android 13+.

Not in it yet: playing in the background, a media notification, lock-screen
and headset controls, the media library, anything remembered between starts.
