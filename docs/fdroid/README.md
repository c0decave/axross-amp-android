# axross-amp-android on F-Droid

Two ways onto F-Droid:

- **A. The official F-Droid repository** (f-droid.org): F-Droid builds the app
  from source itself, from a recipe in their `fdroiddata` repository. This
  folder has the recipe draft, checked with fdroidserver; the merge request
  is yours to open from your GitLab account.
- **B. Our own F-Droid repository** (c0decave.github.io/fdroid): our signed
  APKs from the GitHub releases, available at once. [repo/](repo/) is the
  whole repository to put on GitHub (c0decave/fdroid).

The store texts and pictures for both (and for other stores) are in
[fastlane/metadata/android/](../../fastlane/metadata/android/) (en-US, de-DE),
in the Fastlane layout F-Droid reads from the source.

## Versions

`versionCode` = major * 10000 + minor * 100 + patch, from Cargo.toml's
version (android/app/build.gradle.kts): 0.1.0 is 100, 0.1.1 is 101, 1.2.3
is 10203. It grows with every release as long as minor and patch stay below
100. One APK per release (arm64-v8a), so no per-ABI codes. Every release
needs `fastlane/metadata/android/en-US/changelogs/<versionCode>.txt` (500
characters at most; de-DE too) and a tag `vX.Y.Z`.

## A. Official inclusion (fdroiddata)

### What was checked here

| Policy | State |
|---|---|
| Licence | Apache-2.0 (the app and axross-amp). Every crate linked into the APK is FLOSS: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, MPL-2.0 (symphonia), Unicode-3.0, Unlicense, 0BSD, CC0 (one crate, self_cell, is "Apache-2.0 OR GPL-2.0-only": taken as Apache-2.0); the fonts are Ubuntu Font Licence 1.0, OFL 1.1, MIT. [THIRD_PARTY_LICENSES.md](../../THIRD_PARTY_LICENSES.md), made for the Android target. |
| Gradle dependencies | Only the Android Gradle Plugin: no library (no AndroidX, no Google Play Services, no Firebase, no GMS). The Java side is two classes on the framework. |
| Network | None: no INTERNET permission (CI checks the merged manifest with aapt2), no network crate in the Android build (CI checks `cargo tree`). |
| Binaries in the source | No jar (no gradle-wrapper.jar at all: F-Droid's gradlew-fdroid reads `android/gradle/wrapper/gradle-wrapper.properties`), no prebuilt library. Two one-second test fixtures, `tests/media/sine.mp3` and `sine.flac` (a 440 Hz tone, used only by `cargo test`), the README's PNGs and the store pictures. The demo music of the screenshots is made at run time by `scripts/ci/demo_audio.py`, the icon and feature graphic by `scripts/store/art.py`. libopus is compiled from the opusic-sys crate's C source. |
| Submodule | `axross-amp` at 0e24e3a, which is on the public `main` of c0decave/axross-amp (merged on 2026-11-27, 754c2ec). |
| Maven repositories | `android/settings.gradle.kts` lists Google's Maven Central mirror first (for CI); F-Droid's scanner does not allow it, so the recipe removes the line in `prebuild` (Maven Central and Google stay). |
| Anti-features | None expected. (No ads, no tracking, no network, no non-free dependency or asset.) The photosensitivity warning is in the description. |

`fdroid readmeta`, `fdroid rewritemeta` and `fdroid lint` (fdroidserver
2.4.6) pass on [io.github.c0decave.axrossamp.yml](io.github.c0decave.axrossamp.yml);
the file is in rewritemeta's form.

### Not tested here (no f-droid.org, gitlab.com or Android SDK in this sandbox)

- `fdroid build -v -l io.github.c0decave.axrossamp` - the real build on
  F-Droid's build server image. Check in particular:
  - the `rustup` srclib's version (`srclibs/rustup.yml` in fdroiddata; the
    recipe says `rustup@1.28.2`) and that Rust 1.100.0 is what built the
    release (CI installs the then-current stable);
  - `ndk: r28c` = 28.2.13676358, the NDK in `android/gradle.properties`;
  - Gradle 8.14.6 known to gradlew-fdroid;
  - the categories against fdroiddata's `config/categories.yml`
    (`Local Media Player`, `Multimedia`).
- fdroiddata's CI (it runs lint, the scanner and a build on the MR).

### What you do (the merge request)

Following F-Droid's [Quick Start Guide](https://f-droid.org/en/docs/Submitting_to_F-Droid_Quick_Start_Guide/):

```sh
# 1. On gitlab.com: fork https://gitlab.com/fdroid/fdroiddata (your account).
git clone --depth=1 https://gitlab.com/<you>/fdroiddata ~/fdroiddata
cd ~/fdroiddata
git checkout -b io.github.c0decave.axrossamp

# 2. The recipe
cp /path/to/axross-amp-android/docs/fdroid/io.github.c0decave.axrossamp.yml metadata/

# 3. fdroidserver (or the docker image registry.gitlab.com/fdroid/fdroidserver:buildserver)
python3 -m venv ~/fdroidvenv && ~/fdroidvenv/bin/pip install fdroidserver
export PATH=~/fdroidvenv/bin:$PATH

# 4. The checks the guide asks for
fdroid readmeta
fdroid rewritemeta io.github.c0decave.axrossamp   # must leave the file unchanged
fdroid lint io.github.c0decave.axrossamp
fdroid build -v -l io.github.c0decave.axrossamp   # needs the Android SDK + NDK r28c, or the docker image

# 5. Commit and push, then open the merge request on gitlab.com against
#    fdroid/fdroiddata master, with the "App inclusion" template
git add metadata/io.github.c0decave.axrossamp.yml
git commit -m "New app: axross-amp"
git push -u origin io.github.c0decave.axrossamp
```

The merge request's title and description, filled in from fdroiddata's
template: [merge-request.md](merge-request.md). It stays a draft until the
reproducible-build question below is settled: once F-Droid signs an app
with its own key, it cannot switch to ours later. Answer the reviewers' questions
there; they may ask for the test fixtures to be generated at build time
(then: a small script making the two sine files, and `rm:` for them in the
recipe).

After inclusion, `AutoUpdateMode: Version` and `UpdateCheckMode: Tags`
make F-Droid build every new tag `vX.Y.Z` by itself (the new tag needs its
changelog file and the version in Cargo.toml).

### Reproducible builds (later)

If F-Droid's build of a tag is byte for byte our release APK (signature
aside), F-Droid ships our signed APK, and users can update between GitHub,
our repository and F-Droid. The recipe then gets

```yaml
Binaries: https://github.com/c0decave/axross-amp-android/releases/download/v%v/axross-amp-android-%v-arm64-v8a.apk
AllowedAPKSigningKeys: 914f2e01954356f9a8c463512779cbd2ad21d0de7cb3c6ef068de06426978d2a
```

How close we are (not measured yet - that needs two builds in CI, see
[reproducible.yml](reproducible.yml), a workflow draft not yet in
`.github/`):

- Java and the APK's packing (AGP 8.13, zip times fixed): reproducible.
- The NDK, cargo-ndk and Gradle are pinned. Rust is not: CI installs the
  current stable. A `rust-toolchain.toml` pinning the version is needed.
- Paths: the Rust library carries the build's absolute paths (panic
  locations of the crates in `~/.cargo/registry`, and of the app's own
  source). F-Droid builds in `/home/vagrant/build/io.github.c0decave.axrossamp`,
  CI in `/home/runner/work/...`. Fix: `--remap-path-prefix` for the
  checkout and for `CARGO_HOME` in the Rust flags the Gradle task passes
  (after checking how cargo-ndk 4 combines them with its own), or cargo's
  `trim-paths` once it is stable.
- `SOURCE_DATE_EPOCH`: F-Droid sets it from the commit; nothing in the
  build reads the time as far as known.
- libopus (C, built by cmake through opusic-sys): `-ffile-prefix-map` may be
  needed for the same path reason.
- The licence assets (THIRD_PARTY_LICENSES.md) are files of the commit:
  reproducible.

## B. Our own repository (c0decave/fdroid)

[repo/](repo/) is that repository's content:

| File | What |
|---|---|
| `apps.conf` | the apps: GitHub repository, APK name pattern, signing certificate's SHA-256 (axross-amp-android 914f2e01…8d2a, the companion fa7336c9…110d) |
| `scripts/fetch.py` | downloads every release's APK and SHA256SUMS, checks the sum and the signing certificate (else the run fails), leaves debug APKs out, takes the Fastlane texts from the newest tag (or the default branch) |
| `metadata/*.yml` | licence, source, issue tracker, categories |
| `config.yml` | fdroidserver's configuration; the key from secrets |
| `scripts/site.py` | the Pages site: `repo/`, a page with the address, the fingerprint (read from the key) and a QR code |
| `publish.workflow.yml` (there: `.github/workflows/publish.yml`) | every 6 hours, by hand, or on a `repository_dispatch` "release": fetch, `fdroid update`, sign, publish to Pages. Actions pinned by commit, fdroidserver 2.4.6 from `requirements.txt` with hashes |

Checked here: fetch.py against the real releases (both apps' APKs match
their SHA256SUMS and certificates). Not run here: `fdroid update` (it needs
a JDK) and the workflow itself.

### What is needed (lead / user)

1. **The repository:** a public GitHub repository `c0decave/fdroid` with
   the content of `docs/fdroid/repo/` (with `publish.workflow.yml` moved to
   `.github/workflows/publish.yml`); Settings - Pages - Source: GitHub
   Actions; an environment `fdroid` (Settings - Environments) for the key's
   secrets. Pushing its workflow file needs the usual Approvals click.
2. **The key** (made outside this agent, like the release keys):

   ```sh
   mkdir -p /work/fdroid-repo-key && cd /work/fdroid-repo-key && umask 077
   keytool -genkeypair -keystore keystore.p12 -storetype PKCS12 \
       -alias fdroidrepo -keyalg RSA -keysize 4096 -validity 10000 \
       -dname "CN=c0decave F-Droid repository"
   keytool -list -v -keystore keystore.p12 -alias fdroidrepo | sed -n 's/.*SHA256: //p' | tr -d ':' > fingerprint.txt
   base64 -w0 keystore.p12 | gh secret set FDROID_KEYSTORE_BASE64 -R c0decave/fdroid --env fdroid
   gh secret set FDROID_KEYSTORE_PASS -R c0decave/fdroid --env fdroid   # the store password
   gh secret set FDROID_KEY_PASS -R c0decave/fdroid --env fdroid        # the same for PKCS12
   ```

   Keep a backup of the keystore: a lost key means every user has to add
   the repository again.
3. Run the workflow once (Actions - publish - Run workflow). The site then
   shows the address, the fingerprint and the QR code:
   **https://c0decave.github.io/fdroid/** - users add
   `https://c0decave.github.io/fdroid/repo` in F-Droid (Settings -
   Repositories - +) and check the fingerprint. Put the fingerprint into
   the README's F-Droid section once it exists.
4. Optional: a fine-grained token with "Contents: read and write" on
   c0decave/fdroid as a secret of axross-amp-android's release environment,
   and a last step in the release workflow sending
   `gh api repos/c0decave/fdroid/dispatches -f event_type=release` - else
   the six-hourly run picks a release up.

An app in both repositories (ours and F-Droid's) is only updated across
them when the signatures match: F-Droid's own builds are signed by
F-Droid's key unless the build is reproducible (above).
