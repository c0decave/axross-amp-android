#!/usr/bin/env python3
"""Fetch every release APK of the apps in apps.conf into repo/, checked.

    GH_TOKEN=... python3 -I scripts/fetch.py apps.conf repo/ metadata/

For each GitHub release (drafts left out; pre-releases in) of each app: the
APK matching the glob and SHA256SUMS are downloaded with gh; the APK must be
listed in SHA256SUMS with that sum and signed by the certificate named in
apps.conf (fdroidserver reads the signature), else the run fails. Debug APKs
(*-debug.apk) never go in. The newest release's Fastlane metadata (from the
tag's source) goes to metadata/<appid>/.
"""
import fnmatch
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile


def gh(*args):
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def certificate_sha256(apk):
    from fdroidserver import common
    common.config = common.config or {}
    return hashlib.sha256(common.get_first_signer_certificate(apk)).hexdigest()


def appid_of(apk):
    from fdroidserver import common
    return common.get_apk_id(apk)[0]


def main():
    conf, repo, metadata = sys.argv[1:4]
    os.makedirs(repo, exist_ok=True)
    os.makedirs(metadata, exist_ok=True)
    for line in open(conf):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        project, glob, cert = line.split()
        releases = json.loads(gh("api", f"repos/{project}/releases?per_page=100"))
        newest = None
        for rel in releases:
            if rel["draft"]:
                continue
            names = [a["name"] for a in rel["assets"]]
            apks = [n for n in names if fnmatch.fnmatch(n, glob) and not n.endswith("-debug.apk")]
            if not apks or "SHA256SUMS" not in names:
                print(f"{project} {rel['tag_name']}: no APK or no SHA256SUMS, left out")
                continue
            with tempfile.TemporaryDirectory() as tmp:
                gh("release", "download", rel["tag_name"], "-R", project, "-D", tmp,
                   "-p", "SHA256SUMS", *sum((["-p", a] for a in apks), []))
                sums = {}
                for row in open(os.path.join(tmp, "SHA256SUMS")):
                    digest, name = row.split(None, 1)
                    sums[name.strip().lstrip("*")] = digest
                for apk in apks:
                    path = os.path.join(tmp, apk)
                    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
                    if sums.get(apk) != digest:
                        sys.exit(f"{project} {rel['tag_name']}: {apk} does not match SHA256SUMS")
                    signer = certificate_sha256(path)
                    if signer != cert:
                        sys.exit(f"{project} {rel['tag_name']}: {apk} is signed by {signer}, not {cert}")
                    shutil.copy(path, os.path.join(repo, apk))
                    print(f"{project} {rel['tag_name']}: {apk} ok")
                    if newest is None:
                        newest = (rel["tag_name"], appid_of(path))
        if newest:
            tag, appid = newest
            fetch_fastlane(project, tag, os.path.join(metadata, appid))


def fetch_fastlane(project, tag, target):
    """The Fastlane metadata (fastlane/metadata/android/) as metadata/<appid>/:
    the newest release's, or the default branch's where that tag has none."""
    for ref in (tag, None):
        with tempfile.TemporaryDirectory() as tmp:
            branch = ["--branch", ref] if ref else []
            subprocess.run(["git", "-c", "advice.detachedHead=false", "clone", "-q", "--depth", "1",
                            *branch, f"https://github.com/{project}.git", tmp], check=True)
            source = os.path.join(tmp, "fastlane", "metadata", "android")
            if os.path.isdir(source):
                shutil.rmtree(target, ignore_errors=True)
                shutil.copytree(source, target)
                print(f"{project} {ref or 'default branch'}: Fastlane metadata")
                return
    print(f"{project}: no Fastlane metadata")


if __name__ == "__main__":
    main()
