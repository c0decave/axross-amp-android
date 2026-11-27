# c0decave's F-Droid repository

The signed release APKs of [axross-amp-android](https://github.com/c0decave/axross-amp-android)
and the [axross Android companion](https://github.com/c0decave/axross-android-companion),
for the [F-Droid](https://f-droid.org) client: the same files as on their
GitHub releases, checked against each release's `SHA256SUMS` and the apps'
signing certificates before they go in.

## Add it

In the F-Droid client: Settings - Repositories - **+**, then the address
(or open https://c0decave.github.io/fdroid/ on the phone and tap the link,
or scan its QR code):

```
https://c0decave.github.io/fdroid/repo
```

Check that the client shows this fingerprint (SHA-256 of the repository's
signing certificate):

```
C3 65 C3 B1 AA 63 65 6E 4A C0 92 81 49 D4 D2 07 77 09 04 01 07 76 68 EA 95 C3 BD 15 4C 28 33 1B
```

## How it is made

`.github/workflows/publish.yml` runs every six hours and by hand:
`scripts/fetch.py` downloads every release APK of the apps in `apps.conf`
(debug APKs never) and refuses any whose SHA-256 is not the release's or
whose signing certificate is not the one named there; `fdroid update`
(fdroidserver, pinned with hashes in `requirements.txt`) indexes them and
signs the index with the repository's key (from the environment `fdroid`'s
secrets); `scripts/site.py` writes the page with the address, the
fingerprint read from the key, and a QR code; GitHub Pages publishes it.
Nothing binary is kept in this repository.

The texts and pictures come from each app's `fastlane/metadata/android/`.
