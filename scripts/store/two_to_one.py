"""Store screenshots no longer than 2:1 (Google Play's and others' limit) from
the Fastlane ones: tall shots lose the status bar and the gesture bar
(85 px at the top, 75 at the bottom of 1600), wide ones their sides.

    uv run --with pillow python3 -I scripts/store/two_to_one.py \\
        fastlane/metadata/android/en-US/images/phoneScreenshots docs/stores/screenshots
"""
import os
import sys

from PIL import Image

src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
for name in sorted(os.listdir(src)):
    if not name.endswith(".png"):
        continue
    im = Image.open(os.path.join(src, name))
    w, h = im.size
    if h > 2 * w:
        top = 85 * h // 1600
        im = im.crop((0, top, w, top + 2 * w))
    elif w > 2 * h:
        side = (w - 2 * h) // 2
        im = im.crop((side, 0, side + 2 * h, h))
    im.save(os.path.join(out, name), optimize=True)
    print(name, im.size)
