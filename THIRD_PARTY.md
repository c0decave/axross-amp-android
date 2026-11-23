# Third parties

axross-amp-android itself is Apache-2.0 ([LICENSE](LICENSE)).

## What is in the APK

- **axross-amp** (Apache-2.0): the player - engine, analysis, faces, flash
  guard - from <https://github.com/c0decave/axross-amp>, at the commit the
  `axross-amp` submodule records.
- **Rust crates**, linked into `libaxrossamp.so`: every crate, its licence
  and the licence texts it carries are in
  [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). That file is made by
  axross-amp's own script for the Android target, not edited by hand:

  ```sh
  python3 axross-amp/scripts/third_party_licenses.py \
      --target aarch64-linux-android --output THIRD_PARTY_LICENSES.md
  ```

  Make it again whenever the submodule or `Cargo.lock` changes. Among them,
  only on Android: android-activity, ndk, ndk-sys, ndk-context, jni (all MIT
  OR Apache-2.0) - the glue between the activity and the Rust side.
- **libopus** (BSD-3-Clause, Xiph.Org and contributors): the Opus decoder,
  a C library compiled from the `opusic-sys` crate's bundled source with the
  NDK. Its licence is in THIRD_PARTY_LICENSES.md under opusic-sys.
- **Fonts**, from egui's `epaint_default_fonts`: Ubuntu Light (Ubuntu Font
  Licence 1.0), Hack (MIT), Noto Emoji (SIL OFL 1.1), emoji-icon-font (MIT).
  Their licences are in THIRD_PARTY_LICENSES.md as well.
- **No Java or Kotlin library.** The Java side is two classes of this
  repository on the Android framework (`android.app.NativeActivity`); there
  is no AndroidX, no Kotlin standard library, no Play services.

The three files - LICENSE, this one, THIRD_PARTY_LICENSES.md - are packed
into the APK as `assets/licenses/`.

## What only builds it

Not in the APK: the Android Gradle Plugin and Gradle (Apache-2.0), the
Android SDK and NDK (their own licence terms, accepted where they are
installed), cargo-ndk (Apache-2.0 OR MIT), CMake (BSD-3-Clause), and the
build scripts and proc macros of the crates above.

## The launcher icon

Drawn for this repository (five spectrum bars in the Classic scheme's
colours); Apache-2.0 like the rest.
