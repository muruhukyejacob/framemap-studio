# Generates the FrameMap Studio icon set: icon.png (1024), icon.ico, icon.icns, icon_256.png
from PIL import Image, ImageDraw
import math, sys
S = 4096  # supersampled canvas
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
# gradient background (indigo -> teal), rounded square
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
c1, c2 = (49, 46, 129), (13, 148, 136)
for y in range(S):
    t = y / S
    gd.line([(0, y), (S, y)], fill=tuple(int(c1[i] + (c2[i] - c1[i]) * t) for i in range(3)) + (255,))
mask = Image.new("L", (S, S), 0)
m = int(S * 0.06)
ImageDraw.Draw(mask).rounded_rectangle([m, m, S - m, S - m], radius=int(S * 0.2), fill=255)
img.paste(grad, (0, 0), mask)
d = ImageDraw.Draw(img)
W = (255, 255, 255, 255)
AMBER = (251, 191, 36, 255)
lw = int(S * 0.026)

def arrow(p, q, col, head=True):
    d.line([p, q], fill=col, width=lw)
    if head:
        ang = math.atan2(q[1] - p[1], q[0] - p[0]); L = S * 0.07; a = 0.45
        pts = [q, (q[0] - L * math.cos(ang - a), q[1] - L * math.sin(ang - a)),
               (q[0] - L * math.cos(ang + a), q[1] - L * math.sin(ang + a))]
        d.polygon(pts, fill=col)

# nodes
A = (S * 0.27, S * 0.30); B = (S * 0.73, S * 0.30); C = (S * 0.50, S * 0.74)
nw, nh = S * 0.22, S * 0.15
# connectors (drawn before nodes)
arrow((A[0] + nw / 2, A[1]), (B[0] - nw / 2 - S * 0.01, B[1]), W)       # A -> B
arrow((A[0], A[1] + nh / 2), (C[0] - nw * 0.30, C[1] - nh / 2 - S * 0.012), W)  # A -> C
# B -> C straight connector
p0 = (B[0], B[1] + nh / 2); p1 = (C[0] + nw * 0.30, C[1] - nh / 2 - S * 0.012)
arrow(p0, p1, W)
# horizontal amber line that HOPS over both white connectors
y_cross = S * 0.53
def x_at(pa, pb, y):
    t = (y - pa[1]) / (pb[1] - pa[1]); return pa[0] + (pb[0] - pa[0]) * t
qa = (A[0], A[1] + nh / 2); qb = (C[0] - nw * 0.30, C[1] - nh / 2 - S * 0.012)
xs = sorted([x_at(qa, qb, y_cross), x_at(p0, p1, y_cross)])
r = S * 0.05
pts = [(S * 0.12, y_cross)]
for xc in xs:
    pts.append((xc - r, y_cross))
    for i in range(1, 40):
        th = math.pi * i / 40
        pts.append((xc - r * math.cos(th), y_cross - r * math.sin(th)))
    pts.append((xc + r, y_cross))
pts.append((S * 0.88, y_cross))
d.line(pts, fill=AMBER, width=lw, joint="curve")
for (cx, cy), col, txt in [(A, W, None), (B, W, None), (C, AMBER, None)]:
    d.rounded_rectangle([cx - nw / 2, cy - nh / 2, cx + nw / 2, cy + nh / 2], radius=S * 0.035, fill=col)
# small "text" bars inside nodes
for (cx, cy), col in [(A, (49, 46, 129, 255)), (B, (49, 46, 129, 255)), (C, (120, 53, 15, 255))]:
    d.rounded_rectangle([cx - nw * 0.30, cy - S * 0.012, cx + nw * 0.30, cy + S * 0.012], radius=S * 0.01, fill=col)

out = sys.argv[1]
big = img.resize((1024, 1024), Image.LANCZOS)
big.save(f"{out}/icon.png")
big.resize((256, 256), Image.LANCZOS).save(f"{out}/icon_256.png")
big.save(f"{out}/icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
big.save(f"{out}/icon.icns")
print("ok")
