Draft: New app: axross-amp

<!-- The description of the merge request to fdroid/fdroiddata (template "Default"). -->

## Abstract for new app inclusion

### Please introduce the functions and usage of your app in summary, and also attach some screenshots.

axross-amp is a local music player with 69 visualizations ("faces"):
spectra, scopes, meters, a waterfall, a synthwave sun, a galaxy and more,
full screen, moving with the music. Music comes from a folder or files
picked with Android's Storage Access Framework (no storage permission);
it plays on in the background with a media notification and lock-screen
controls; the list and position come back at the next start. MP3, FLAC,
Ogg Vorbis, Opus, AAC/ALAC, WAV, AIFF, CAF, MKA, gapless.

The app has **no network permission and no network code**: no tracking, no
ads, no update check, nothing downloaded.

Screenshots: https://github.com/c0decave/axross-amp-android/tree/main/fastlane/metadata/android/en-US/images/phoneScreenshots

### Please elaborate why you wrote this app. Please highlight the unique functions it provides over other similar apps in F-Droid.

It is the Android build of axross-amp, a desktop player written in Rust
(symphonia, cpal, egui, wgpu): the same library crate, used as a git
submodule, with its visualizations and its flash guard (WCAG 2.3.1/2.3.2:
no more than three flashes a second, no saturated red flash). Few players
on F-Droid draw the music full screen in this many ways; this one does it
entirely offline.

## Checklist

### Policy

* [x] The app complies with the inclusion policy: Apache-2.0; every Rust crate in the APK is FLOSS (list: https://github.com/c0decave/axross-amp-android/blob/main/THIRD_PARTY_LICENSES.md); the only Gradle dependency is the Android Gradle Plugin (no AndroidX, no Play Services, no Firebase); no binaries in the source (no gradle-wrapper.jar; two one-second test tones used by `cargo test` only); no network. `fdroid scanner`'s source scan finds nothing once the recipe drops the Maven mirror line from settings.gradle.kts.
* [x] The original app author has been notified: this MR is from the author.
* [x] The upstream repo has Fastlane metadata (en-US, de-DE): summary, description, icon, feature graphic, screenshots, changelogs.

### Docs

* [x] Read the guide, the templates, the Build Metadata Reference and the Quick Start Guide.

### Merge Request Setup

* [x] Title "New app: axross-amp" (a draft until the reproducible-build question below is settled).
* [x] The fork is public, the branch is not protected.
* [x] No related fdroiddata or RFP issues.
* [x] One app in this MR.

### Metadata

* [x] `metadata/io.github.c0decave.axrossamp.yml`, valid YAML, LF line endings; `fdroid rewritemeta` leaves it unchanged, `fdroid lint` passes.
* [x] No summary/description/images in fdroiddata: they come from upstream's Fastlane folder.
* [x] Releases are tagged (`vX.Y.Z`); AutoUpdateMode Version, UpdateCheckMode Tags. The first build is the commit that added the Fastlane metadata (after the v0.1.1 tag; the app's code is v0.1.1's); the next release will be a tag.
* [x] Issue tracker and AuthorName.
* [x] The external repo (axross-amp, the player library) is a git submodule (`submodules: true`), pinned to a commit on its public main.
* [ ] Reproducible builds: wanted - the release APKs on GitHub are signed by us (certificate SHA-256 914f2e01954356f9a8c463512779cbd2ad21d0de7cb3c6ef068de06426978d2a). Not enabled yet: the Rust library still carries absolute build paths and CI does not pin the Rust version. This MR stays a draft until a release is reproducible; then the recipe gets `Binaries` and `AllowedAPKSigningKeys`.
* [x] One ABI (arm64-v8a), about 14.5 MB: no split needed.
* [x] Only the latest version; no disabled versions.
* [x] `commit` is a full hash.

### Pipeline

* [ ] All pipelines should pass (to be seen here).
* [ ] Warnings in the Reports tab: to be answered here.
