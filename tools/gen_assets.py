"""Generates the TEXembed app icon and in-app UI icons from SVG sources.

Outputs (written to OUT):
  appicon.png, icon.ico            - application / window icon
  open-file.png, open-folder.png,
  logs.png, about.png, tweaks.png  - monochrome rail icons (white + alpha),
                                     tinted at runtime by egui's painter.image
"""
import io
import os
import cairosvg
from PIL import Image

OUT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SVG_DIR = os.path.join(OUT, "tools", "svg")
os.makedirs(SVG_DIR, exist_ok=True)

# ---------------------------------------------------------------- app logo
# Texture-tile motif: a rounded dark tile, a T bar that spans over an E whose
# arms end in diagonal cuts (echoing texel / mip edges), and a faint texel grid.
def logo_svg(texels=True):
    grid = ""
    if texels:
        cells = []
        size = 28
        for r in range(4):
            for c in range(4):
                if (r + c) % 2 == 0:
                    x = 512 - 40 - size * 4 + c * size
                    y = 512 - 40 - size * 4 + r * size
                    cells.append(f'<rect x="{x}" y="{y}" width="{size}" height="{size}"/>')
        grid = ('<g clip-path="url(#tile)" fill="#FFFFFF" fill-opacity="0.045">'
                + "".join(cells) + "</g>")
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
<defs>
  <clipPath id="tile"><rect x="16" y="16" width="480" height="480" rx="112"/></clipPath>
  <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#1D1D22"/><stop offset="1" stop-color="#0C0C0F"/>
  </linearGradient>
  <linearGradient id="or" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#FDB170"/><stop offset="1" stop-color="#E96F25"/>
  </linearGradient>
  <linearGradient id="bl" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#8FC0FC"/><stop offset="1" stop-color="#4A8EF2"/>
  </linearGradient>
  <radialGradient id="glow" cx="0.5" cy="0.42" r="0.6">
    <stop offset="0" stop-color="#60A5FA" stop-opacity="0.16"/>
    <stop offset="1" stop-color="#60A5FA" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect x="16" y="16" width="480" height="480" rx="112" fill="url(#bg)"/>
<rect x="16" y="16" width="480" height="480" rx="112" fill="url(#glow)"/>
{grid}
<rect x="17" y="17" width="478" height="478" rx="111" fill="none"
      stroke="#FFFFFF" stroke-opacity="0.10" stroke-width="2"/>
<!-- T: top bar with a diagonal right cut, plus stem -->
<path d="M96 132 H416 L384 188 H96 Z" fill="url(#or)"/>
<rect x="136" y="188" width="60" height="196" rx="0" fill="url(#or)"/>
<path d="M136 188 H196 V384 H136 Z" fill="#000" fill-opacity="0.0"/>
<!-- E: spine + three arms with diagonal cuts -->
<rect x="224" y="208" width="56" height="176" fill="url(#bl)"/>
<path d="M280 208 H416 L388 252 H280 Z" fill="url(#bl)"/>
<path d="M280 274 H368 L340 318 H280 Z" fill="url(#bl)"/>
<path d="M280 340 H416 L388 384 H280 Z" fill="url(#bl)"/>
</svg>'''

def mark_svg():
    """Borderless TE monogram that fills the canvas (for window/taskbar/exe icons)."""
    body = logo_svg(False)
    start = body.index("<!-- T:")
    end = body.index("</svg>")
    defs = body[body.index("<defs>"):body.index("</defs>") + 7]
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="88 90 336 336">'
            + defs + body[start:end] + "</svg>")

# ---------------------------------------------------------------- UI icons
# 24px grid, 2px round strokes. Secondary shapes use reduced alpha so the
# runtime tint (grey / white / blue) keeps a two-tone look.
STROKE = 'fill="none" stroke="#FFFFFF" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"'
SOFT = 'fill="#FFFFFF" fill-opacity="0.38" stroke="none"'
SOLID = 'fill="#FFFFFF" stroke="none"'

ICONS = {
    "open-file": f'''
<path d="M14 3H7.5A2.5 2.5 0 0 0 5 5.5v13A2.5 2.5 0 0 0 7.5 21h9a2.5 2.5 0 0 0 2.5-2.5V8z" {STROKE}/>
<path d="M14 3v3.8A1.2 1.2 0 0 0 15.2 8H19" {STROKE}/>
<rect x="8.6" y="12" width="3" height="3" rx="0.6" {SOLID}/>
<rect x="11.6" y="15" width="3" height="3" rx="0.6" {SOLID}/>
<rect x="11.6" y="12" width="3" height="3" rx="0.6" {SOFT}/>
<rect x="8.6" y="15" width="3" height="3" rx="0.6" {SOFT}/>''',
    "open-folder": f'''
<path d="M3.5 18.5V6A2 2 0 0 1 5.5 4h3.3a2 2 0 0 1 1.6.8l.9 1.2h5.2a2 2 0 0 1 2 2v1.5" {STROKE}/>
<path d="M3.5 18.5l2.1-6.3A2 2 0 0 1 7.5 10.8H20a1.4 1.4 0 0 1 1.35 1.8l-1.75 5.6a2 2 0 0 1-1.9 1.4H5.5a2 2 0 0 1-2-1.1z" {STROKE}/>
<path d="M9 14.4h6.4" stroke="#FFFFFF" stroke-opacity="0.38" stroke-width="1.9" stroke-linecap="round" fill="none"/>''',
    "logs": f'''
<rect x="3" y="4.5" width="18" height="15" rx="3.2" {STROKE}/>
<path d="M7.4 9.6l3 2.4-3 2.4" {STROKE}/>
<path d="M13 14.8h3.8" {STROKE}/>''',
    "about": f'''
<circle cx="12" cy="12" r="9" {STROKE}/>
<circle cx="12" cy="7.9" r="1.25" {SOLID}/>
<path d="M12 11.2v5.3" {STROKE}/>''',
    "tweaks": f'''
<path d="M4 6.5h3.2M12.2 6.5H20" {STROKE}/>
<path d="M4 12h8.2M17.2 12H20" {STROKE}/>
<path d="M4 17.5h2M11 17.5h9" {STROKE}/>
<circle cx="9.7" cy="6.5" r="2.5" {SOFT}/><circle cx="9.7" cy="6.5" r="2.5" {STROKE}/>
<circle cx="14.7" cy="12" r="2.5" {SOFT}/><circle cx="14.7" cy="12" r="2.5" {STROKE}/>
<circle cx="8.5" cy="17.5" r="2.5" {SOFT}/><circle cx="8.5" cy="17.5" r="2.5" {STROKE}/>''',
}

def icon_svg(body):
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">'
            + body + "</svg>")

def render(svg, size):
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=size, output_height=size)
    return Image.open(io.BytesIO(png)).convert("RGBA")

def main():
    # App icon: borderless mark so it stays large at 16-32px; tile variant kept for docs/stores
    mark = mark_svg()
    tile = logo_svg(True)
    open(os.path.join(SVG_DIR, "appicon.svg"), "w").write(mark)
    open(os.path.join(SVG_DIR, "appicon-tile.svg"), "w").write(tile)
    render(mark, 512).save(os.path.join(OUT, "appicon.png"))
    render(tile, 512).save(os.path.join(OUT, "appicon-tile.png"))
    sizes = [16, 24, 32, 48, 64, 128, 256]
    frames = [render(mark, s) for s in sizes]
    frames[-1].save(os.path.join(OUT, "icon.ico"), format="ICO",
                    sizes=[(s, s) for s in sizes], append_images=frames[:-1])

    # UI icons (128px: crisp when downsampled to ~22px, esp. with mipmaps)
    for name, body in ICONS.items():
        svg = icon_svg(body)
        open(os.path.join(SVG_DIR, f"{name}.svg"), "w").write(svg)
        render(svg, 128).save(os.path.join(OUT, f"{name}.png"))
    print("done")

if __name__ == "__main__":
    main()
