# Changelog

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
