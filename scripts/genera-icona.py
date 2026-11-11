#!/usr/bin/env python3
"""Genera l'icona 1024x1024 di Code Art (vetro scuro su aurora).

Uso: python3 scripts/genera-icona.py [percorso.png]
Poi: pnpm tauri icon <percorso.png>
"""
import sys
from PIL import Image, ImageDraw, ImageFilter, ImageChops

S = 1024
out = sys.argv[1] if len(sys.argv) > 1 else "icon-source.png"


def blob(color, cx, cy, r):
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(layer).ellipse((cx - r, cy - r, cx + r, cy + r), fill=color)
    return layer.filter(ImageFilter.GaussianBlur(r * 0.45))


def rounded_mask(box, radius):
    m = Image.new("L", (S, S), 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius, fill=255)
    return m


# sfondo e aurora
bg = Image.new("RGBA", (S, S), "#0a0b12")
for col, cx, cy, r in [
    ((0x3B, 0x2A, 0x8C, 255), 280, 260, 380),
    ((0x0F, 0x6B, 0x6B, 255), 760, 330, 340),
    ((0x7A, 0x2D, 0x5C, 255), 560, 820, 360),
]:
    bg = Image.alpha_composite(bg, blob(col, cx, cy, r))

# quadrato arrotondato (griglia icone macOS: 824 px centrati)
box = (100, 100, 924, 924)
icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
icon.paste(bg, mask=rounded_mask(box, 185))

# lastra di vetro
pane_box = (210, 230, 814, 794)
pane_mask = rounded_mask(pane_box, 96)
glass = Image.new("RGBA", (S, S), (255, 255, 255, 0))
grad = Image.new("L", (S, S), 0)
gd = ImageDraw.Draw(grad)
for y in range(S):
    gd.line([(0, y), (S, y)], fill=int(46 - 26 * (y - 230) / 564) if 230 <= y <= 794 else 0)
glass.putalpha(ImageChops.multiply(grad, pane_mask))
icon = Image.alpha_composite(icon, glass)
d = ImageDraw.Draw(icon)
d.rounded_rectangle(pane_box, 96, outline=(255, 255, 255, 70), width=4)
# riflesso in alto
hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(hl).rounded_rectangle((226, 244, 798, 420), 84, fill=(255, 255, 255, 38))
hl.putalpha(ImageChops.multiply(hl.getchannel("A"), pane_mask))
icon = Image.alpha_composite(icon, hl)

# fumetto con cursore
g = Image.new("RGBA", (S, S), (0, 0, 0, 0))
gd = ImageDraw.Draw(g)
white = (240, 243, 255, 235)
gd.rounded_rectangle((300, 340, 724, 620), 80, fill=white)
gd.polygon([(360, 600), (360, 710), (480, 610)], fill=white)
# cursore a barra, ritagliato nel fumetto
gd.rounded_rectangle((470, 410, 506, 550), 16, fill=(10, 11, 18, 255))
gd.rounded_rectangle((540, 520, 640, 550), 15, fill=(10, 11, 18, 255))
shadow = g.filter(ImageFilter.GaussianBlur(18))
sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
sh.putalpha(shadow.getchannel("A").point(lambda v: v // 3))
icon = Image.alpha_composite(icon, sh)
icon = Image.alpha_composite(icon, g)
icon.save(out)
print("scritto", out)
