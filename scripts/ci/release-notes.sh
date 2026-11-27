#!/usr/bin/env bash
# A release's notes (release.yml, job publish):
#
#   scripts/ci/release-notes.sh VERSION > notes.md
#
# The CHANGELOG's section for VERSION, then what the two APKs are, then the
# photosensitivity warning. Fails when the CHANGELOG has no such section.
set -euo pipefail

VERSION=${1:?the version}
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

notes=$(awk -v v="$VERSION" '
    $0 ~ "^## " v " " { on = 1; next }
    on && /^## / { exit }
    on { print }
' "$ROOT/CHANGELOG.md")
[ -n "$notes" ] || { echo "::error::no '## $VERSION' section in CHANGELOG.md" >&2; exit 1; }

printf '%s\n' "$notes"
cat <<EOF

### The files

- \`axross-amp-android-$VERSION-arm64-v8a.apk\` - the app: signed with the
  release key (certificate SHA-256 in the README), for arm64 phones,
  Android 9 or later.
- \`axross-amp-android-$VERSION-arm64-v8a-debug.apk\` - the same commit as a
  debug build, for reports and testing: debuggable, signed with the build's
  debug key (not the release key, and a new one at every release: uninstall
  the last debug APK before installing a new one), installed as an app of
  its own beside the release one (the package
  \`io.github.c0decave.axrossamp.debug\`, the same name on the home screen,
  the version \`$VERSION-debug\`), with more in its log (\`adb logcat -s
  axrossamp\`: track names and positions too) and the test hooks CI uses
  (music handed over by adb). Not for everyday use.
- \`SHA256SUMS\` - both APKs' SHA-256 sums: \`sha256sum -c SHA256SUMS\`.
EOF
printf '\n**Photosensitivity warning**: the visualizations move with the music. They are guarded against flashes and patterns (see the README), but the pictures are not guaranteed safe for everyone; if you or anyone watching has epilepsy, do not use them.\n'
