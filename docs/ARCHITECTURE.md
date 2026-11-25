# Architecture

How axross-amp-android is put together, why, and what step 2 builds on.

## The shape

```
axross-amp-android/
├── Cargo.toml, Cargo.lock      one crate: libaxrossamp.so (cdylib) + rlib
├── axross-amp/                 git submodule: the desktop program (the player)
├── src/
│   ├── lib.rs                  android_main: logging, wgpu's setup, eframe
│   ├── screen.rs               the one screen (eframe::App): face + controls
│   ├── player.rs               engine + playlist for picked tracks; no window
│   ├── transport.rs            the engine's commands from several threads
│   ├── source.rs               opens tracks off the window's thread
│   ├── opener.rs               content:// document -> a path the engine opens
│   ├── picked.rs, gesture.rs   the pickers' answers; swipes (std only)
│   ├── platform/               the seam to Android, and its host stand-in
│   └── bin/preview.rs          the same screen in a desktop window
├── android/                    a plain Gradle project (AGP only)
│   └── app/src/main/           manifest, two Java classes, resources
├── scripts/                    smoke_test.sh (emulator), check_pins.py,
│                               ci/: what the CI's apk and smoke jobs run
├── security/                   the supply-chain gate's configuration
└── .github/                    ci.yml, release.yml, the shared toolchain action
```

### Reuse, not a copy

The player is `axross_amp` - the desktop repository's library crate - as a
path dependency on a git submodule. This crate calls its `engine`,
`playlist`, `analyzer`, `meters` and `viz` exactly as the desktop's window
does: `Engine::start`, `Analyzer::update`, `viz::draw`. The flash guard runs
inside `viz::draw`; there is no second implementation and no switch for it
here.

What the desktop crate needed to compile for Android is one change, on its
`android` branch: `eframe` with default features everywhere except Android,
where it comes with `default_fonts` and `wgpu` only (eframe refuses its
`accesskit` feature together with a NativeActivity; the clipboard, links,
X11 and Wayland have no place there). Everything else - cpal's `pulseaudio`
feature, zbus, MPRIS, the mixer, the file dialog, the updater, detaching -
already compiles for Android or is already behind `target_os = "linux"`,
and what the Android app never calls is dropped by the linker. So no
feature gates were added: fewer of them is less to keep in step.

### Rust and Java

The Java side is as small as it can be: `MainActivity` (a `NativeActivity`
that loads the library and forwards the lifecycle) and `Bridge` (the
pickers, reading folders, opening documents, the insets, audio focus). They
talk through **one method each way**, so there are two JNI signatures to
get right and no more:

- Rust → Java: `int MainActivity.bridgeCall(int what, String arg)`
- Java → Rust: `static native void Bridge.nativeEvent(int kind, int a, int b, int c, int d, String text)`

The numbers are constants with the same names in `Bridge.java` and
`src/platform/mod.rs`. Step 2's service and MediaSession use the same two
methods with new numbers (`EVENT_MEDIA` and the `MEDIA_*` commands are
already there and already handled by the screen).

Events arrive on Java's threads, possibly while no frame is drawn (the
activity is stopped). What must not wait - pausing when the app is hidden,
on a loss of audio focus, when headphones are pulled - is done at once by a
hook (`Transport::interrupt`); the rest is queued for the next frame.

### content:// documents

The engine opens tracks by path and reads them again by path when it seeks.
A picked document has no path. `opener.rs` asks Java for a file descriptor
(`ContentResolver.openFileDescriptor`, detached) and names it in the app's
private files (`files/open/N.ext`) with a symbolic link to
`/proc/self/fd/N`: the engine then opens the very file the provider handed
over, without a copy, and without one line changed in the desktop's engine.
Where such a link cannot be opened (a pipe, some cloud providers), the
document is copied instead. Six tracks stay open at most. All of it happens
on the `amp-source` thread: opening a file never holds up a frame.

Whether re-opening `/proc/self/fd/N` is allowed for every provider on every
Android version is not known until it runs on devices; the copy is the
net under it.

## Decisions

**NativeActivity, not GameActivity (for now).** Both work with winit's
android-activity. NativeActivity is part of the framework: no AndroidX
library in the APK, no C++ glue to compile, nothing whose version must match
the Rust crate's - the fewest things that can go wrong in a build nobody
could run locally. Its cost: no AccessKit (TalkBack) and a poor soft
keyboard. The proof has no text input; accessibility matters and is the
reason to move to GameActivity in step 2 - a change of one Cargo feature in
each repository (`android-game-activity`, and eframe's defaults back on),
`MainActivity`'s base class, and the `androidx.games:games-activity`
dependency in the version android-activity names. `Bridge` does not depend
on the activity's base class for that reason.

**cargo-ndk + Gradle, not xbuild or cargo-apk.** cargo-apk is deprecated by
its authors. xbuild builds APKs without Gradle - which is the point of it,
and the problem: step 2 needs a service, a MediaSession, a notification and
probably AndroidX Media3, i.e. Java sources, a manifest and Maven
dependencies, which is what a Gradle project is for. cargo-ndk only sets up
the NDK's compilers for cargo and copies the `.so` files into `jniLibs/`
shape; it is the usual tool for this, small, and replaceable by a dozen
environment variables should it ever fall behind (its latest release is
4.1.2). The Gradle project is ordinary: `gradle -p android assembleDebug`
does everything (it runs `cargo ndk` itself; `-PskipCargo` leaves that out).

**Java, not Kotlin.** Two small classes. Kotlin would add a Gradle plugin
and its standard library to the APK and the supply chain for no gain here.

**wgpu: Vulkan, then OpenGL ES.** Both backends are compiled in; wgpu picks
Vulkan where the device offers it. The device's limits are requested
(`downlevel_defaults` at the adapter's resolution), not a desktop's
defaults - a phone that offers 4096-pixel textures still gets a device.
`adb shell setprop debug.axrossamp.backend gl|vulkan|auto` forces one; the
smoke test uses that to fall back on an emulator whose Vulkan does not work.

**The music pauses when the app is hidden.** See the README. Technically the
engine could keep playing for a while; it would be killed without warning.

**The process ends when the activity does.** winit cannot create a second
event loop in a process, and Android may start the activity again in the
same one. `android_main` therefore exits the process when the window is
gone; the next start is a fresh one. (With step 2's service the player
moves out of the activity's lifetime - see below.)

**Application id `io.github.c0decave.axrossamp`** (debug builds add
`.debug`). It follows the repository's home; nobody else can claim that
name's domain. Should the app ever go to a store under an organisation's
domain, the id would have to change *before* the first public release - an
installed app cannot change it.

**minSdk 26** (Android 8.0): AAudio, which cpal uses; adaptive icons;
`AudioFocusRequest`. **targetSdk / compileSdk 36.**

### Versions

| What | Version | Why this one |
|---|---|---|
| eframe / egui / wgpu / winit / cpal / symphonia | the desktop's `Cargo.lock` | one reviewed set of crates |
| android-activity | 0.6.1 (the lock's) | what winit 0.30 uses |
| Android Gradle Plugin | 8.13.0 | supports compileSdk 36; a known pairing with Gradle 8.14 |
| Gradle | 8.14.6 | the 8.x line AGP 8.13 was made for |
| NDK | 28.2.13676358 (r28c) | 16 KB pages by default |
| JDK | 17 | what AGP 8 runs on |
| cargo-ndk | 4.1.2 | the latest release |

AGP 8.13.0, the NDK's and the emulator images' availability could not be
checked where this was written (Google's repositories were not reachable);
they are the first things to look at if the first CI run fails.

## Supply chain

- **Crates**: axross's gate, as on the desktop, on this repository's
  `Cargo.lock`. The reviewed baseline is the desktop's, read from the
  submodule (`security/supply-chain-cargo.toml`). This works because the
  lock here was made from the desktop's and holds nothing else;
  `scripts/check_pins.py` fails the gate when that stops being true.
- **Actions**: pinned by commit SHA with a version comment; checked by the
  same script.
- **The player**: a submodule commit.
- **Gradle's dependencies** (AGP and its tree): *not verified yet*. Gradle
  can verify them by checksum (`android/gradle/verification-metadata.xml`);
  the file has to be made where Google's Maven repository is reachable. The
  `apk` job makes it on each run until one is committed and offers it as the
  artifact `gradle-verification-metadata`: review it, commit it, and Gradle
  enforces it from then on.
- **Tools in CI**: rustup's stable toolchain, cargo-ndk 4.1.2 from crates.io
  (`--locked`: its own lock, not gated), Gradle from the Gradle action, the
  JDK from the Java action, the SDK/NDK from the runner's image or
  `sdkmanager`. As on the desktop, these are trusted as distributed.

## Step 2

What step 1 leaves in place for it, and what it takes.

1. **The player out of the activity.** Today `Screen` owns `Player`. For
   playback to outlive the activity, the player has to live in the process,
   not in the window: a `static` player service in Rust (the `Player` as it
   is - it knows no window) that the screen attaches to when there is one.
   `android_main` then no longer exits the process while music plays; the
   event loop's "one per process" limit is met by keeping the loop alive
   across activity restarts or by ending the process only when nothing
   plays. This is the one structural change, and `player.rs`,
   `transport.rs` and `source.rs` were written for it.
2. **Foreground service + media notification.** A `PlaybackService`
   (`foregroundServiceType="mediaPlayback"`), started by `Bridge` when
   playback starts and stopped when it ends for good; permissions
   `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MEDIA_PLAYBACK`,
   `POST_NOTIFICATIONS` (asked at the first play). The notification is a
   `MediaStyle` one fed from the session.
3. **MediaSession**: lock screen, headset and Bluetooth buttons, Android
   Auto's basics. Its callbacks call `Bridge.event(EVENT_MEDIA, MEDIA_*)` -
   already dispatched to the player; the other way, new `CALL_*` numbers
   carry the metadata (title, artist, album, duration, cover) and the
   playback state on every change. Framework `MediaSession` needs no
   library; Media3's `MediaSession` + `MediaSessionService` is the
   recommended path and brings AndroidX in (then also decide GameActivity).
   Audio focus and "becoming noisy" move from `Bridge` into the service.
4. **GameActivity + AccessKit** (TalkBack), with the move to AndroidX.
5. **Phone layout**: a now-playing screen with the cover, the queue as a
   list, the faces as a pager with thumbnails, settings (scheme, motion,
   frame rate, the equalizer - all exist in the desktop crate's state),
   Android's "remove animations" mapped to the analyzer's reduced motion,
   immersive mode while the controls are hidden. The desktop's `state`
   module is reused for what is remembered (the list, the face, the
   position), stored in the app's files directory.
6. **Media library**: `READ_MEDIA_AUDIO` and MediaStore (albums, artists,
   tracks with their content URIs - `opener.rs` plays them as it plays
   picked documents); the picked folders remembered with persisted URI
   permissions.
7. **Release channels**: the signed APK on GitHub releases (in place);
   F-Droid (reproducible-build metadata, no Google dependencies - true
   today), then Google Play if wanted (an AAB, Play signing, the id fixed
   before that). An in-app licences screen showing `assets/licenses/`.
8. **Per-ABI APKs** and armeabi-v7a if 32-bit devices matter.
