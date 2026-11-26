# Changelog

## 0.1.1 (unreleased)

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
- New launcher icon: the Synthwave Sun, with a themed-icon layer for
  Android 13+.

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

Not in it yet: playing in the background, a media notification, lock-screen
and headset controls, the media library, anything remembered between starts.
