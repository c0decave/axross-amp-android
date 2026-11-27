# Aptoide listing: axross-amp

Everything to fill in at Aptoide Connect (connect.aptoide.com) for
`io.github.c0decave.axrossamp`. The texts are the Fastlane ones
(`fastlane/metadata/android/`), as plain text.

## The app

| Field | Value |
|---|---|
| Package | `io.github.c0decave.axrossamp` |
| Name | axross-amp |
| Category | Music & Audio (Aptoide: "Music & Audio"; if offered as a subcategory: Music Players) |
| Price | Free |
| Developer | c0decave |
| Website | https://github.com/c0decave/axross-amp-android |
| Support | https://github.com/c0decave/axross-amp-android/issues |
| Privacy policy URL | https://github.com/c0decave/axross-amp-android/blob/main/docs/PRIVACY.md |
| Licence | Apache-2.0 (open source) |

## Texts

| | English | German |
|---|---|---|
| Title | axross-amp | axross-amp |
| Short description (80) | `fastlane/metadata/android/en-US/short_description.txt`: "Music player with 69 visualizations - fully offline, no network permission" | `de-DE/short_description.txt`: "Musikplayer mit 69 Visualisierungen - komplett offline, ohne Netzwerkrecht" |
| Long description | [description-en.txt](description-en.txt) | [description-de.txt](description-de.txt) |
| What's new (0.1.1) | `fastlane/metadata/android/en-US/changelogs/101.txt` | `de-DE/changelogs/101.txt` |

Keywords / tags (where asked): music player, visualizer, spectrum, offline,
FLAC, Opus, no ads, open source.

## Content rating and data safety answers

| Question | Answer |
|---|---|
| Violence, sexual content, profanity, drugs, gambling, horror | None |
| User-generated content, chat, sharing between users | No |
| Ads | No |
| In-app purchases / payments | No |
| Data collection, analytics, tracking | None (no network permission at all) |
| Data shared with third parties | None |
| Account / login | No |
| Location, contacts, camera, microphone | Not used |
| Internet access | No (the APK has no INTERNET permission) |
| Target audience | Everyone; not designed for children specifically |
| Flashing lights | The visualizations move with the music; they are guarded against flashes (WCAG 2.3.1/2.3.2) and the description carries a photosensitivity warning. Mention it where the rating asks about flashing images. |

Expected rating: everyone / PEGI 3 equivalent (where a questionnaire such
as IARC asks about flashing images, answer as above).

## Pictures

| What | File | Size |
|---|---|---|
| Icon | [icon-512.png](icon-512.png) | 512 x 512 PNG |
| Feature graphic | [feature-graphic-1024x500.png](feature-graphic-1024x500.png) | 1024 x 500 PNG |
| Phone screenshots (portrait) | [../screenshots/](../screenshots/) 01, 02, 05-09 | 720 x 1440 PNG (2:1) |
| Phone screenshots (details) | ../screenshots/ 03 (notification), 04 (lock screen) | 720 x 672 / 736 |
| Landscape screenshot | ../screenshots/10_landscape-controls.png | 1440 x 720 |

The icon and the feature graphic are drawn by `scripts/store/art.py`, the
screenshots come from CI's emulator (`scripts/store/screenshots.py`,
`scripts/store/two_to_one.py`); demo music made up by a script, nothing
personal on them.

## The APK

`axross-amp-android-<version>-arm64-v8a.apk` from
https://github.com/c0decave/axross-amp-android/releases - not the
`-debug.apk`. Check it first: `sha256sum -c --ignore-missing SHA256SUMS`.

| | |
|---|---|
| Format | APK (no AAB; Aptoide installs APKs) |
| Signature | APK Signature Scheme v2 + v3, release key, certificate SHA-256 `914f2e01954356f9a8c463512779cbd2ad21d0de7cb3c6ef068de06426978d2a` - every update must carry the same signature |
| minSdk / targetSdk | 26 / 36 (supported from Android 9, API 28) |
| ABIs | arm64-v8a only (64-bit) |
| versionCode | 101 for 0.1.1 (major * 10000 + minor * 100 + patch) |
| Size | about 14.5 MB |
