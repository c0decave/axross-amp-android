#!/usr/bin/env python3
"""The Pages site: site/repo/ (what fdroid update made) and site/index.html,
a page with the repository's address, its fingerprint and a QR code to add
it in the F-Droid client.

    FDROID_KEYSTORE_PASS=... python3 -I scripts/site.py keystore.p12 repo site

The fingerprint is the SHA-256 of the repository key's certificate (what
the F-Droid client shows and checks); it is read from the keystore here,
never set by hand.
"""
import hashlib
import html
import os
import shutil
import sys

import qrcode
import qrcode.image.svg
from cryptography.hazmat.primitives.serialization import Encoding, pkcs12

URL = "https://c0decave.github.io/fdroid/repo"


def main():
    keystore, repo, site = sys.argv[1:4]
    password = os.environ["FDROID_KEYSTORE_PASS"].encode()
    _, cert, _ = pkcs12.load_key_and_certificates(open(keystore, "rb").read(), password)
    fingerprint = hashlib.sha256(cert.public_bytes(Encoding.DER)).hexdigest().upper()
    link = f"{URL}?fingerprint={fingerprint}"
    shutil.rmtree(site, ignore_errors=True)
    shutil.copytree(repo, os.path.join(site, "repo"))
    qr = qrcode.make(link, image_factory=qrcode.image.svg.SvgPathImage, box_size=8)
    qr.save(os.path.join(site, "qr.svg"))
    pairs = " ".join(fingerprint[i:i + 2] for i in range(0, len(fingerprint), 2))
    open(os.path.join(site, "fingerprint.txt"), "w").write(fingerprint + "\n")
    open(os.path.join(site, "index.html"), "w").write(f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>c0decave's F-Droid repository</title>
<style>body{{font-family:system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;line-height:1.5}}
code{{word-break:break-all}} img{{width:14rem;height:14rem;background:#fff}}</style></head>
<body>
<h1>c0decave's F-Droid repository</h1>
<p>The signed release APKs of <a href="https://github.com/c0decave/axross-amp-android">axross-amp-android</a>
and the <a href="https://github.com/c0decave/axross-android-companion">axross Android companion</a> - the same files
as on their GitHub releases, checked against each release's SHA256SUMS and the apps' signing certificates.</p>
<h2>Add it</h2>
<p>Open this page on the phone and tap the link, or scan the QR code in the F-Droid client
(Settings - Repositories - +):</p>
<p><a href="{html.escape(link)}"><img src="qr.svg" alt="QR code of the repository's address with its fingerprint"></a></p>
<p>Address: <code>{URL}</code></p>
<p>Fingerprint (SHA-256 of the repository's signing certificate) - check that the client shows this one:<br>
<code>{pairs}</code></p>
</body></html>
""")
    print(f"fingerprint {fingerprint}")


if __name__ == "__main__":
    main()
