#!/usr/bin/env python3
"""Regenerate the Android launcher icons from the 512px master.

    python3 scripts/gen-android-icons.py --preview   # contact sheet, writes nothing
    python3 scripts/gen-android-icons.py             # writes into gen/android res/

Source of truth is `src-tauri/icons/icon.png`: the full-bleed Penrose mark,
already bounding-box centred on its own canvas.

Two things this fixes, both visible on a home screen:

CENTRING. The mark's alpha-weighted centre of mass sits dead centre while its
BOUNDING BOX sits 23px (5.75dp) to the right, because a Penrose triangle carries
a heavy column on the left and thin extensions to the right. The previous
foreground was mass-centred, and mass-centring is the wrong metric for a compact
geometric emblem: the eye reads the silhouette's extent, not its weight, so it
read as shoved right with a visibly fat left margin. Everything here centres on
the BOUNDING BOX.

BACKGROUND. Brushed gunmetal rather than the flat violet gradient it replaced
(which itself replaced a flat #fff). The brush is real anisotropic noise, not a
gradient pretending: high-frequency variation across the grain, almost none
along it, rotated to the brush angle. Kept DARK on purpose. The mark is a
mid steel blue (#4a6b8a-ish), so a bright silver ground would swallow it, which
is the same trap the original #fff fell into.
"""

import argparse
import os
from PIL import Image, ImageChops, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
MASTER = os.path.join(ROOT, 'src-tauri', 'icons', 'icon.png')
RES = os.path.join(ROOT, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'res')

# Adaptive layers are a 108dp canvas of which only the middle 72dp is guaranteed
# visible; the outer ring is reserved for mask and parallax.
ADAPTIVE_DP = 108
# Height of the mark inside that canvas. Matches what shipped, so this change is
# centring and colour only, not a resize.
ART_DP = 56

DENSITIES = {'mdpi': 1, 'hdpi': 1.5, 'xhdpi': 2, 'xxhdpi': 3, 'xxxhdpi': 4}
LEGACY_DP = 48

# Gunmetal ramp, darkest valley to brightest crest of the grain. Deliberately
# dark: the mark averages ~#4a6b8a, so the plate has to stay well under it or
# the emblem stops reading. Bright silver looks great in isolation and loses the
# logo completely, which is the same mistake the original #fff ground made.
METAL_LOW = (0x0D, 0x0F, 0x13)
METAL_HIGH = (0x3C, 0x43, 0x4E)
# Spread of the grain. High values read as coarse smear rather than brushed.
GRAIN_SIGMA = 34
# Angle of the brush, degrees counter-clockwise.
BRUSH_ANGLE = 22
# How far the sheen lifts the face where the light catches it.
SHEEN_GAIN = 34


def _grain(size: int, seed_size: int) -> Image.Image:
    """Anisotropic noise: fine across the grain, smeared along it.

    Built at 2x and resolved down at the end. Generating at output resolution
    gives one streak per pixel row, which at icon size reads as a coarse
    diagonal smudge rather than brushed metal; supersampling puts two streaks in
    the space of one and lets the downsample antialias them into fine hairlines.
    """
    ss = seed_size * 2
    # Few columns, many rows -> every row gets its own value while a row barely
    # varies along its length. Upscaling smears that into streaks.
    noise = Image.effect_noise((max(2, ss // 24), ss), GRAIN_SIGMA)
    noise = noise.resize((ss, ss), Image.BICUBIC)
    noise = noise.filter(ImageFilter.GaussianBlur(0.4))
    noise = noise.rotate(BRUSH_ANGLE, resample=Image.BICUBIC)
    off = (ss - size * 2) // 2
    noise = noise.crop((off, off, off + size * 2, off + size * 2))
    return noise.resize((size, size), Image.LANCZOS)


def _linear_ramp(size: int, angle_from_top_left: bool = True) -> Image.Image:
    """Soft diagonal ramp used for the sheen and the depth falloff."""
    ramp = Image.linear_gradient('L').resize((size, size), Image.BILINEAR)
    ramp = ramp.rotate(-45 if angle_from_top_left else 135, resample=Image.BICUBIC)
    return ramp.filter(ImageFilter.GaussianBlur(size / 12))


def brushed_metal(size: int) -> Image.Image:
    """Brushed gunmetal plate, `size` square, fully opaque."""
    grain = _grain(size, int(size * 1.6))

    # Colourise the grain across the gunmetal ramp.
    plate = Image.new('RGB', (size, size))
    px = plate.load()
    gp = grain.load()
    for y in range(size):
        for x in range(size):
            t = gp[x, y] / 255.0
            px[x, y] = tuple(
                int(METAL_LOW[i] + (METAL_HIGH[i] - METAL_LOW[i]) * t) for i in range(3)
            )

    # Light catching the plate: a diagonal band, brightest top-left. Squared
    # rather than linear so it reads as a defined highlight raking across metal
    # instead of a flat wash from one corner to the other.
    sheen = _linear_ramp(size)
    lift = Image.new('RGB', (size, size))
    lp = lift.load()
    sp = sheen.load()
    for y in range(size):
        for x in range(size):
            t = 1.0 - sp[x, y] / 255.0
            v = int(SHEEN_GAIN * t * t)
            lp[x, y] = (v, v, v)
    plate = ImageChops.add(plate, lift)

    # Vignette. White in the mask is the plate as-is, black is the darkened
    # copy, so the ellipse has to be the region that stays LIT and the corners
    # fall away. Getting this backwards lights the centre and pushes the plate
    # bright exactly where the mark needs a dark bed.
    vig = Image.new('L', (size, size), 0)
    ImageDraw.Draw(vig).ellipse(
        (-size * 0.30, -size * 0.30, size * 1.30, size * 1.30), fill=255
    )
    vig = vig.filter(ImageFilter.GaussianBlur(size / 5))
    dark = Image.new('RGB', (size, size), (0, 0, 0))
    plate = Image.composite(plate, Image.blend(plate, dark, 0.55), vig)
    return plate


# Brand accent, matching --color-accent in src/styles/globals.css.
ACCENT = (0x97, 0xB1, 0xB9)


def _radial(size: int, inner: float, outer: float) -> Image.Image:
    """White-to-black radial falloff mask."""
    m = Image.new('L', (size, size), 0)
    d = ImageDraw.Draw(m)
    steps = 48
    for i in range(steps, 0, -1):
        t = i / steps
        r = size * (inner + (outer - inner) * t) / 2
        c = size / 2
        d.ellipse((c - r, c - r, c + r, c + r), fill=int(255 * (1 - t)))
    return m.filter(ImageFilter.GaussianBlur(size / 14))


def _mix(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def _diagonal(size: int, top: tuple, bottom: tuple) -> Image.Image:
    """Linear diagonal gradient between two colours."""
    ramp = Image.new('RGB', (size, size))
    px = ramp.load()
    for y in range(size):
        for x in range(size):
            px[x, y] = _mix(top, bottom, (x + y) / (2.0 * size))
    return ramp


def background(style: str, size: int) -> Image.Image:
    """Candidate launcher grounds. All stay well under the mark's luminance:
    it is a mid steel blue, and anything light swallows it."""
    if style == 'brushed':
        return brushed_metal(size)

    if style == 'ink':
        # Flat near-black. The mark carries the whole icon.
        return Image.new('RGB', (size, size), (0x0E, 0x10, 0x13))

    if style == 'glow':
        # Near-black with a soft accent bloom sitting behind the mark.
        base = Image.new('RGB', (size, size), (0x0B, 0x0D, 0x10))
        bloom = Image.new('RGB', (size, size), _mix((0x0B, 0x0D, 0x10), ACCENT, 0.30))
        return Image.composite(bloom, base, _radial(size, 0.05, 1.5))

    if style == 'slate':
        # Monochromatic with the mark: deep slate falling to near-black.
        return _diagonal(size, (0x22, 0x2C, 0x36), (0x0A, 0x0D, 0x11))

    if style == 'teal':
        # Deep teal-navy duotone, cooler and richer than the slate.
        return _diagonal(size, (0x10, 0x33, 0x40), (0x07, 0x0D, 0x14))

    if style == 'ring':
        # Near-black with one faint concentric ring, like a lens barrel.
        base = Image.new('RGB', (size, size), (0x0C, 0x0E, 0x12))
        d = ImageDraw.Draw(base)
        for i, alpha in ((0.86, 0.22), (0.62, 0.12)):
            r = size * i / 2
            c = size / 2
            d.ellipse((c - r, c - r, c + r, c + r),
                      outline=_mix((0x0C, 0x0E, 0x12), ACCENT, alpha),
                      width=max(1, size // 108))
        return base.filter(ImageFilter.GaussianBlur(size / 300 + 0.4))

    raise ValueError(f'unknown background style: {style}')


def monochrome(scale: float) -> Image.Image:
    """Themed-icon layer for Android 13+.

    The launcher flat-tints this to the wallpaper palette and supplies its own
    background, so it must be a SILHOUETTE, not our colour art. Pointing
    <monochrome> at the colour foreground (which is what shipped) collapses the
    Penrose mark to a solid blob, because every cube face is fully opaque and
    only the outer edge has any alpha at all - the illusion is carried entirely
    by the three face shades, and flat tinting throws all three away.

    So map LUMINANCE onto ALPHA. Alpha survives tinting, which keeps the top,
    left and right faces distinguishable and the impossible triangle readable in
    a single colour.
    """
    canvas = round(ADAPTIVE_DP * scale)
    art = mark(round(ART_DP * scale)).convert('RGBA')
    px = art.load()
    for y in range(art.height):
        for x in range(art.width):
            r, g, b, a = px[x, y]
            if not a:
                continue
            lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0
            px[x, y] = (255, 255, 255, int(a * (0.34 + 0.66 * min(1.0, lum * 1.35))))
    return centred(art, canvas)


def mark(height_px: int) -> Image.Image:
    """The Penrose mark, cropped to its own bounds and scaled to `height_px`."""
    src = Image.open(MASTER).convert('RGBA')
    src = src.crop(src.getchannel('A').getbbox())
    w = max(1, round(src.width * height_px / src.height))
    return src.resize((w, height_px), Image.LANCZOS)


def centred(art: Image.Image, canvas: int) -> Image.Image:
    """Bounding-box centred on a transparent square. See the module docstring
    for why this is not centre-of-mass."""
    out = Image.new('RGBA', (canvas, canvas), (0, 0, 0, 0))
    out.paste(art, ((canvas - art.width) // 2, (canvas - art.height) // 2), art)
    return out


def foreground(scale: float) -> Image.Image:
    canvas = round(ADAPTIVE_DP * scale)
    return centred(mark(round(ART_DP * scale)), canvas)


def legacy(scale: float, round_mask: bool, style: str) -> Image.Image:
    """Pre-composited icon for launchers older than adaptive icons."""
    size = round(LEGACY_DP * scale)
    plate = background(style, size).convert('RGBA')
    # Legacy icons have no mask overhead, so the mark can take more of the face.
    art = mark(round(size * 0.62))
    plate.alpha_composite(art, ((size - art.width) // 2, (size - art.height) // 2))
    if round_mask:
        m = Image.new('L', (size, size), 0)
        ImageDraw.Draw(m).ellipse((0, 0, size - 1, size - 1), fill=255)
        plate.putalpha(m)
    return plate


def write_all(style: str) -> None:
    for name, scale in DENSITIES.items():
        d = os.path.join(RES, f'mipmap-{name}')
        os.makedirs(d, exist_ok=True)
        foreground(scale).save(os.path.join(d, 'ic_launcher_foreground.png'))
        # Separate asset, NOT the colour foreground. See monochrome().
        monochrome(scale).save(os.path.join(d, 'ic_launcher_monochrome.png'))
        legacy(scale, False, style).save(os.path.join(d, 'ic_launcher.png'))
        legacy(scale, True, style).save(os.path.join(d, 'ic_launcher_round.png'))

        db = os.path.join(RES, f'drawable-{name}')
        os.makedirs(db, exist_ok=True)
        background(style, round(ADAPTIVE_DP * scale)).save(
            os.path.join(db, 'ic_launcher_background.png')
        )
        print(f'  {name:<8} foreground + monochrome + background + 2 legacy')


def write_preview(path: str) -> None:
    """Contact sheet: adaptive stack under each mask, plus the centring check."""
    size = 216
    pad = 20
    tiles = []

    fg = foreground(2)
    bg = background(DEFAULT_STYLE, size).convert("RGBA")

    full = bg.copy()
    full.alpha_composite(fg)
    tiles.append(('square (full 108dp)', full))

    for label, shape in (('circle mask', 'ellipse'), ('squircle mask', 'round')):
        m = Image.new('L', (size, size), 0)
        d = ImageDraw.Draw(m)
        if shape == 'ellipse':
            d.ellipse((0, 0, size - 1, size - 1), fill=255)
        else:
            d.rounded_rectangle((0, 0, size - 1, size - 1), radius=size * 0.26, fill=255)
        t = full.copy()
        t.putalpha(m)
        tiles.append((label, t))

    # Centring check: old mass-centred foreground beside the new one, with the
    # canvas centre line drawn through both.
    old_path = os.path.join(RES, 'mipmap-xxhdpi', 'ic_launcher_foreground.png')
    for label, img in (('BEFORE (on disk)', Image.open(old_path).convert('RGBA')
                        .resize((size, size), Image.LANCZOS)),
                       ('AFTER (bbox centred)', fg.copy())):
        t = Image.new('RGBA', (size, size), (26, 26, 30, 255))
        t.alpha_composite(img)
        d = ImageDraw.Draw(t)
        d.line((size // 2, 0, size // 2, size), fill=(255, 90, 90, 255), width=1)
        bb = img.getchannel('A').getbbox()
        if bb:
            d.rectangle((bb[0], bb[1], bb[2] - 1, bb[3] - 1), outline=(90, 220, 120, 255))
        tiles.append((label, t))

    cols = len(tiles)
    sheet = Image.new('RGBA', (cols * size + (cols + 1) * pad, size + pad * 2 + 22),
                      (18, 18, 22, 255))
    d = ImageDraw.Draw(sheet)
    for i, (label, t) in enumerate(tiles):
        x = pad + i * (size + pad)
        sheet.alpha_composite(t, (x, pad))
        d.text((x, pad + size + 6), label, fill=(200, 200, 210, 255))
    sheet.save(path)
    print(f'preview -> {path}')


STYLES = ['ink', 'glow', 'slate', 'teal', 'ring', 'brushed']
# Chosen by eye off `--options`. Brushed metal was tried and rejected; so were a
# flat #fff and a violet gradient before it.
DEFAULT_STYLE = 'glow'


def write_options(path: str) -> None:
    """Every candidate ground under the squircle mask, plus the themed layer."""
    size = 200
    pad = 18
    fg = foreground(size / ADAPTIVE_DP)

    def squircle(img):
        m = Image.new('L', (size, size), 0)
        ImageDraw.Draw(m).rounded_rectangle(
            (0, 0, size - 1, size - 1), radius=size * 0.26, fill=255)
        out = img.copy()
        out.putalpha(m)
        return out

    tiles = []
    for s in STYLES:
        t = background(s, size).convert('RGBA')
        t.alpha_composite(fg)
        tiles.append((s, squircle(t)))

    # Themed icon: the launcher flat-tints the monochrome layer onto its own
    # wallpaper-derived ground. Shown twice so the tinting is obviously dynamic.
    mono = monochrome(size / ADAPTIVE_DP)
    for label, bg_c, fg_c in (('themed (light wallpaper)', (0xDA, 0xE2, 0xE6), (0x25, 0x33, 0x3B)),
                              ('themed (dark wallpaper)', (0x2B, 0x33, 0x38), (0xC5, 0xDA, 0xE2))):
        t = Image.new('RGBA', (size, size), bg_c + (255,))
        tint = Image.new('RGBA', (size, size), fg_c + (255,))
        tint.putalpha(mono.getchannel('A'))
        t.alpha_composite(tint)
        tiles.append((label, squircle(t)))

    cols = 4
    rows = (len(tiles) + cols - 1) // cols
    sheet = Image.new('RGBA',
                      (cols * size + (cols + 1) * pad, rows * (size + 26) + pad),
                      (18, 18, 22, 255))
    d = ImageDraw.Draw(sheet)
    for i, (label, t) in enumerate(tiles):
        x = pad + (i % cols) * (size + pad)
        y = pad + (i // cols) * (size + 26)
        sheet.alpha_composite(t, (x, y))
        d.text((x, y + size + 6), label, fill=(205, 205, 215, 255))
    sheet.save(path)
    print(f'options -> {path}')


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--preview', metavar='PATH', nargs='?', const='icon-preview.png')
    ap.add_argument('--options', metavar='PATH', nargs='?', const='icon-options.png')
    ap.add_argument('--style', default=DEFAULT_STYLE, choices=STYLES)
    args = ap.parse_args()
    if args.options:
        write_options(args.options)
    elif args.preview:
        write_preview(args.preview)
    else:
        print('writing launcher icons:')
        write_all(args.style)
        print('done. gen/android is regenerated by `tauri android init` - '
              're-run this script after any re-init.')
