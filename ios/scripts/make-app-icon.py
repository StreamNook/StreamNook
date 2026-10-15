"""Compose the StreamNook iPhone app icon: the logo at 62%, centred, on a coloured base.

The shipped icon is the midnight variant (the app palette: near-black base, slate accent glow).
Regenerate from the transparent logo, from ios/:

    python3 -I scripts/make-app-icon.py ../src-tauri/icons/icon_square.png /tmp midnight
    cp /tmp/AppIcon-midnight.png Resources/AppIcon.png

No names renders every variant (midnight, mist, twilight, ember) for comparison.
"""
import sys, math
from PIL import Image, ImageDraw, ImageFilter, ImageChops

SIZE = 1024
SS = 2  # supersample for smooth gradients

def hexrgb(h):
    h = h.lstrip('#'); return tuple(int(h[i:i+2], 16) for i in (0, 2, 4))

def lerp(a, b, t): return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))

def gradient(top, bottom, size):
    """Diagonal-ish vertical gradient, top-left light to bottom-right dark."""
    w = h = size
    g = Image.new('RGB', (1, 256))
    for y in range(256):
        g.putpixel((0, y), lerp(top, bottom, y / 255))
    g = g.resize((w * 2, h * 2), Image.BILINEAR).rotate(-12, resample=Image.BICUBIC)
    off = w // 2
    return g.crop((off, off, off + w, off + h))

def radial(color, size, cx, cy, radius, strength):
    """Soft glow layer as RGBA."""
    mask = Image.new('L', (size, size), 0)
    d = ImageDraw.Draw(mask)
    d.ellipse((cx - radius, cy - radius, cx + radius, cy + radius), fill=int(255 * strength))
    mask = mask.filter(ImageFilter.GaussianBlur(radius * 0.55))
    layer = Image.new('RGBA', (size, size), color + (0,))
    layer.putalpha(mask)
    return layer

def compose(logo_path, out_path, top, bottom, glow, glow_strength, shadow_alpha, scale=0.62, nudge_x=0.018):
    S = SIZE * SS
    base = gradient(hexrgb(top), hexrgb(bottom), S).convert('RGBA')
    if glow:
        base = Image.alpha_composite(base, radial(hexrgb(glow), S, S // 2, int(S * 0.47), int(S * 0.36), glow_strength))
    logo = Image.open(logo_path).convert('RGBA')
    logo = logo.crop(logo.getbbox())
    target = int(S * scale)
    ratio = target / max(logo.size)
    logo = logo.resize((round(logo.width * ratio), round(logo.height * ratio)), Image.LANCZOS)
    # The mark is a right-pointing triangle, so its visual weight sits left of its box: nudge right.
    x = (S - logo.width) // 2 + int(S * nudge_x)
    y = (S - logo.height) // 2
    if shadow_alpha:
        a = logo.getchannel('A').point(lambda v: int(v * shadow_alpha))
        sh = Image.new('RGBA', logo.size, (0, 0, 0, 0)); sh.putalpha(a)
        layer = Image.new('RGBA', (S, S), (0, 0, 0, 0))
        layer.paste(sh, (x, y + int(S * 0.018)), sh)
        layer = layer.filter(ImageFilter.GaussianBlur(S * 0.022))
        base = Image.alpha_composite(base, layer)
    base.alpha_composite(logo, (x, y))
    # iOS app icons must be fully opaque.
    base.convert('RGB').resize((SIZE, SIZE), Image.LANCZOS).save(out_path, optimize=True)

VARIANTS = {
    # name: top, bottom, glow, glow strength, shadow
    "midnight": ("#26343F", "#0B0F13", "#97B1B9", 0.36, 0.55),
    'mist':     ('#EEF3F5', '#AFC3CB', '#FFFFFF', 0.55, 0.35),
    'twilight': ('#30355A', '#11131F', '#8FA8FF', 0.28, 0.55),
    'ember':    ('#FFE3C7', '#F0A273', '#FFF4E6', 0.60, 0.30),
}

if __name__ == '__main__':
    logo, outdir = sys.argv[1], sys.argv[2]
    names = sys.argv[3:] or list(VARIANTS)
    for n in names:
        compose(logo, f'{outdir}/AppIcon-{n}.png', *VARIANTS[n])
        print('wrote', n)
