# Draws FossCut's app icon: a play triangle split by a diagonal cut on a dark rounded square.
import sys
from PIL import Image, ImageDraw

out = sys.argv[1]
S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([40, 40, S - 40, S - 40], radius=220, fill=(29, 31, 36, 255))
tri = [(330, 250), (330, 774), (790, 512)]
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).polygon(tri, fill=255)
# The cut: a diagonal band removed across the triangle.
ImageDraw.Draw(mask).polygon([(250, 640), (290, 690), (860, 300), (820, 250)], fill=0)
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
for y in range(S):
    t = y / S
    gd.line([(0, y), (S, y)], fill=(int(255 - 20 * t), int(120 + 60 * t), int(60 + 40 * t), 255))
img.paste(grad, (0, 0), mask)
img = img.resize((512, 512), Image.LANCZOS)
img.save(f"{out}/icon.png")
img.save(f"{out}/icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
