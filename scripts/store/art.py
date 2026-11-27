"""The Synthwave Sun (android/app/src/main/res/drawable/ic_launcher_foreground.xml)
as SVG, and from it the store pictures: icon 512 px, feature graphic 1024x500.

    uv run --with cairosvg python3 -I scripts/store/art.py fastlane/metadata/android/en-US/images
"""
import os, sys
import cairosvg

SUN = ("M54,28H54A19,19 0 0,1 72.92,48.7H35.08A19,19 0 0,1 54,28Z M35.24,50H72.76A19,19 0 0,1 72.03,53H35.97A19,19 0 0,1 35.24,50Z "
       "M36.86,55.2H71.14A19,19 0 0,1 69.9,57.4H38.1A19,19 0 0,1 36.86,55.2Z M40.53,60.4H67.47A19,19 0 0,1 66.04,61.7H41.96A19,19 0 0,1 40.53,60.4Z")
GRID = "M26,68H82 M28,71.5H80 M32,76.5H76 M40,83H68 M54,68L54,86 M47,68L41.75,83.75 M61,68L66.25,83.75 M38,68L31.33,76.75 M70,68L76.67,76.75"
BG = "#1B0B33"

def art():
    return f'''<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="54" y1="28" x2="54" y2="66">
<stop offset="0" stop-color="#FFD23F"/><stop offset="1" stop-color="#FF3E8A"/></linearGradient>
<radialGradient id="glow" gradientUnits="userSpaceOnUse" cx="54" cy="48" r="34">
<stop offset="0" stop-color="#FF3E8A" stop-opacity="0.35"/><stop offset="1" stop-color="#FF3E8A" stop-opacity="0"/></radialGradient></defs>
<circle cx="54" cy="48" r="34" fill="url(#glow)"/>
<path d="{SUN}" fill="url(#g)"/>
<path d="{GRID}" fill="none" stroke="#B45CFF" stroke-opacity="0.9" stroke-width="1.4" stroke-linecap="round"/>'''

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
# icon: the adaptive icon's art, cropped to its safe area's neighbourhood
icon = f'<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="17 19 74 74"><rect x="0" y="0" width="108" height="108" fill="{BG}"/>{art()}</svg>'
cairosvg.svg2png(bytestring=icon.encode(), write_to=os.path.join(out, "icon.png"), output_width=512, output_height=512)
# feature graphic: 1024x500, the sun on the left third, the name beside it
feat = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="500" viewBox="0 0 1024 500">
<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#12061F"/><stop offset="1" stop-color="{BG}"/></linearGradient></defs>
<rect width="1024" height="500" fill="url(#bg)"/>
<g transform="translate(-10,-30) scale(5.2)">{art()}</g>
<text x="520" y="235" fill="#FFFFFF" font-family="DejaVu Sans, sans-serif" font-size="72" font-weight="bold">axross-amp</text>
<text x="524" y="300" fill="#D9C2FF" font-family="DejaVu Sans, sans-serif" font-size="30">music, 69 visualizations</text>
<text x="524" y="345" fill="#D9C2FF" font-family="DejaVu Sans, sans-serif" font-size="30">offline, no tracking</text>
</svg>'''
cairosvg.svg2png(bytestring=feat.encode(), write_to=os.path.join(out, "featureGraphic.png"), output_width=1024, output_height=500)
print("ok")
