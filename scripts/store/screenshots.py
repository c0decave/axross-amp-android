"""The store screenshots (fastlane/metadata/android/en-US/images/phoneScreenshots)
from a screenshots run (the `screenshots` branch, made by scripts/ci/screenshots.sh):
cropped, 720 px on the short side, 256 colours.

    uv run --with pillow python3 -I scripts/store/screenshots.py SHOTS_DIR OUT_DIR
"""
import os, sys
from PIL import Image
src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
picks = ["01-player-controls", "18-cover-controls", "13-notification", "14-lock-screen", "11-faces-list",
         "19-face-synthwave", "20-face-galaxy", "21-face-skyline", "03-face", "15-landscape-controls"]
for n, name in enumerate(picks, 1):
    im = Image.open(os.path.join(src, name + ".png")).convert("RGB")
    w, h = im.size
    if name.startswith("13") : im = im.crop((0, 0, w, int(h * 0.42)))
    if name.startswith("14") : im = im.crop((0, 0, w, int(h * 0.46)))
    scale = 720 / min(im.size) if not name.startswith(("13","14")) else 720 / w
    im = im.resize((round(im.width * scale), round(im.height * scale)), Image.LANCZOS)
    im = im.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    label = name.split("-", 1)[1]
    if label == "face": label = "face-mirrored-spectrum"
    p = os.path.join(out, f"{n:02d}_{label}.png")
    im.save(p, optimize=True)
    print(os.path.basename(p), im.size, os.path.getsize(p))
