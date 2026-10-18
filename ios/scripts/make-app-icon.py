"""Compose the StreamNook iPhone app icon: the logo in frosted white at 60%, centred, on a smooth
diagonal gradient of two neighbouring hues.

The shipped icon is the aurora variant (violet to indigo). Icons that stand out on a home screen
pair one clean gradient of neighbouring hues with a light glyph; the slate-blue logo turns muddy
on a vivid field, so the cubes are redrawn in white, each face keeping its shade: lit faces white,
shaded faces a pale tint of the gradient's deep end.

Regenerate from the transparent logo, from ios/:

    python3 -I scripts/make-app-icon.py ../src-tauri/icons/icon_square.png /tmp aurora
    cp /tmp/AppIcon-aurora.png Resources/AppIcon.png

No names renders every variant (aurora, lagoon, sunset, electric, orchid, coral) for comparison.
"""
import sys

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
SS = 2  # supersampled, then scaled down
S = SIZE * SS

VARIANTS = {
    # name: (top-left, bottom-right)
    'aurora': ('#7B6CFF', '#4F46E5'),    # violet to indigo
    'lagoon': ('#2DD4BF', '#2563EB'),    # teal to blue
    'sunset': ('#FF9A3C', '#FF3D77'),    # orange to pink
    'electric': ('#38BDF8', '#6366F1'),  # sky to indigo
    'orchid': ('#C084FC', '#7C3AED'),    # lilac to purple
    'coral': ('#FF7A7A', '#E8467C'),     # coral to raspberry
}


def hexrgb(h):
    h = h.lstrip('#')
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def diagonal(c1, c2, size):
    """Top-left c1 to bottom-right c2, eased so the middle is soft (worked on a small grid,
    scaled up, which keeps it smooth)."""
    n = 256
    grid = Image.new('RGB', (n, n))
    px = grid.load()
    for y in range(n):
        for x in range(n):
            t = (x + y) / (2 * (n - 1))
            t = t * t * (3 - 2 * t)
            px[x, y] = tuple(round(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))
    return grid.resize((size, size), Image.BICUBIC)


def sheen(size, strength):
    """A soft pool of light at the top-left, so the field is not flat."""
    mask = Image.new('L', (size, size), 0)
    r = int(size * 0.55)
    ImageDraw.Draw(mask).ellipse((-r // 2, -r // 2, r, r), fill=int(255 * strength))
    mask = mask.filter(ImageFilter.GaussianBlur(size * 0.18))
    layer = Image.new('RGBA', (size, size), (255, 255, 255, 0))
    layer.putalpha(mask)
    return layer


def percentile(hist, q):
    """The value below which `q` of the histogram's counts fall."""
    total = sum(hist)
    run = 0
    for value, count in enumerate(hist):
        run += count
        if run >= total * q:
            return value
    return 255


def white_logo(logo, deep):
    """The cubes in white, each face keeping its shade: the logo's own light and dark map from
    white (lit faces) down to white mixed 42% toward `deep` (shaded faces)."""
    alpha = logo.getchannel('A')
    lum = logo.convert('L')
    solid = alpha.point(lambda v: 255 if v > 200 else 0)
    hist = lum.histogram(mask=solid)
    lo, hi = percentile(hist, 0.02), percentile(hist, 0.98)
    shade = tuple(int(255 + (c - 255) * 0.42) for c in deep)
    lut = [min(1, max(0, (v - lo) / max(1, hi - lo))) ** 0.8 for v in range(256)]
    channels = [lum.point([int(shade[i] + (255 - shade[i]) * lut[v]) for v in range(256)]) for i in range(3)]
    out = Image.merge('RGB', channels).convert('RGBA')
    out.putalpha(alpha)
    return out


def compose(logo_path, out_path, top, bottom, scale=0.60, nudge=0.018, sheen_strength=0.22, shadow=0.18):
    base = diagonal(hexrgb(top), hexrgb(bottom), S).convert('RGBA')
    base = Image.alpha_composite(base, sheen(S, sheen_strength))
    logo = Image.open(logo_path).convert('RGBA')
    logo = logo.crop(logo.getbbox())
    k = S * scale / max(logo.size)
    logo = logo.resize((round(logo.width * k), round(logo.height * k)), Image.LANCZOS)
    glyph = white_logo(logo, hexrgb(bottom))
    # The mark is a right-pointing triangle, so its visual weight sits left of its box: nudge right.
    x = (S - glyph.width) // 2 + int(S * nudge)
    y = (S - glyph.height) // 2
    if shadow:
        shade = Image.new('RGBA', glyph.size, (0, 0, 0, 0))
        shade.putalpha(glyph.getchannel('A').point(lambda v: int(v * shadow)))
        layer = Image.new('RGBA', (S, S), (0, 0, 0, 0))
        layer.paste(shade, (x, y + int(S * 0.012)), shade)
        base = Image.alpha_composite(base, layer.filter(ImageFilter.GaussianBlur(S * 0.016)))
    base.alpha_composite(glyph, (x, y))
    # iOS app icons must be fully opaque.
    base.convert('RGB').resize((SIZE, SIZE), Image.LANCZOS).save(out_path, optimize=True)


if __name__ == '__main__':
    logo, outdir = sys.argv[1], sys.argv[2]
    for name in sys.argv[3:] or list(VARIANTS):
        compose(logo, f'{outdir}/AppIcon-{name}.png', *VARIANTS[name])
        print('wrote', name)
