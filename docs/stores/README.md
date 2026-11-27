# Stores

| Store | How | Material |
|---|---|---|
| GitHub releases | every tag `vX.Y.Z` (release.yml): signed APK, debug APK, SHA256SUMS | - |
| F-Droid (f-droid.org) | F-Droid builds from source after the fdroiddata merge request | [../fdroid/](../fdroid/) |
| F-Droid repository of our own | our signed APKs, c0decave.github.io/fdroid | [../fdroid/repo/](../fdroid/repo/) |
| Aptoide | uploaded by you in Aptoide Connect | [aptoide/](aptoide/) |

The texts and pictures of all of them come from
`fastlane/metadata/android/` (en-US, de-DE); [screenshots/](screenshots/)
holds the same screenshots cut to 2:1 for stores that want that.
Privacy policy for every store: [../PRIVACY.md](../PRIVACY.md) -
https://github.com/c0decave/axross-amp-android/blob/main/docs/PRIVACY.md

## Aptoide, step by step

What follows is from Aptoide's developer console as it is generally known;
connect.aptoide.com and Aptoide's documentation (docs.catappult.io) could
not be opened from where this was written, so check the exact field names
there.

### Once: the account

1. Go to https://connect.aptoide.com and sign up as a developer (e-mail
   and password, or a Google account). Fill in the developer profile: name
   "c0decave", the website and a support contact (an e-mail address or the
   issue tracker URL).
2. Accept the developer agreement. A free app needs no payment details;
   skip anything about Catappult's in-app billing (the app has no
   purchases).

### The first upload

1. **Apps - Add / Upload app**, then upload
   `axross-amp-android-0.1.1-arm64-v8a.apk` (from the GitHub release,
   checked with `SHA256SUMS`; not the `-debug.apk`).
2. Aptoide reads the package name, the version and the signature from the
   APK. If it says the package is already claimed (someone uploaded a copy
   before), use its ownership claim: Aptoide checks that you hold the key
   the APK is signed with (certificate SHA-256 `914f2e01…8d2a`); the claim
   may ask you to upload an APK they prepare or sign a token with the
   release key - that needs the release keystore, which only you handle.
3. Fill in the listing from [aptoide/listing.md](aptoide/listing.md): name,
   category Music & Audio, the short and long descriptions in English and
   German (add German as a second language), the "what's new" text, the
   website, the support contact, the privacy policy URL.
4. Pictures: the icon (512 x 512), the feature graphic (1024 x 500) and the
   screenshots from [screenshots/](screenshots/) (portrait 720 x 1440 first:
   01, 02, 05, 06, 07; then 03, 04 and the landscape one).
5. Content rating / age questions: the answers in
   [aptoide/listing.md](aptoide/listing.md) (no ads, no tracking, no
   network, no in-app purchases, no user content; flashing images warned
   about).
6. Distribution: all countries; free. Submit for review. Aptoide checks the
   APK (malware scan, signature) before it is public.

### Updates

Every new release: download the new signed APK from the GitHub release,
check it with `SHA256SUMS`, upload it to the same app in Connect ("Upload
new version"), and paste `fastlane/metadata/android/<locale>/changelogs/<versionCode>.txt`
as the "what's new". The signature must stay the same (the release key);
the versionCode grows by itself (major * 10000 + minor * 100 + patch).

### Uploading from CI

Aptoide has had an upload API for developers (Catappult's "publishing
API", with an API key from the developer console). Whether it is
available for this account, and its exact endpoint, has to be looked up in
Connect / docs.catappult.io. If it is: create the key in Connect, store it
as a secret (for example `APTOIDE_API_KEY` in the environment "release"),
and a last step of the release workflow can upload the signed APK. Nothing
of this is set up: it needs your key, and the workflow change needs the
Approvals click.

## What only you can do

- Create the Aptoide Connect account and accept its agreement.
- Upload the APK, fill in the listing, answer the rating questions, submit.
- An ownership claim, if one is needed (it uses the release key).
- An API key for uploads from CI, if wanted.
