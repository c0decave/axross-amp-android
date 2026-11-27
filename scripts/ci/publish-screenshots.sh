#!/usr/bin/env bash
# The screenshots of scripts/ci/screenshots.sh to the branch "screenshots"
# (screenshots.yml): one commit of PNGs only, replacing the branch, from
# which they are picked for docs/screenshots/ by hand.
#
#   GH_TOKEN=... scripts/ci/publish-screenshots.sh SHOTS_DIR
set -euo pipefail

SHOTS=${1:?the screenshots}
: "${GH_TOKEN:?GH_TOKEN is not set}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}"
work=$(mktemp -d)
cp "$SHOTS"/*.png "$work"/
cp "$SHOTS"/app-log.txt "$work"/ 2>/dev/null || true
cd "$work"
git init -q -b screenshots
git add -- *.png app-log.txt 2>/dev/null || git add -- *.png
git -c user.name="github-actions[bot]" -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
    commit -q -m "Screenshots of ${GITHUB_SHA:-a build}"
git push -q --force "https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPOSITORY}.git" screenshots
echo "the screenshots are on the branch screenshots: $(ls ./*.png | wc -l) PNGs"
