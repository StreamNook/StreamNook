// The sharp badge art Penrose makes: a 288 px PNG master and a 144 px WebP, each on
// cdn.streamnook.app named by its own sha256. The index carries them as `a`, the two
// names joined by a dot. This is the app's twin of the site's sharpUrls, and both are
// tested against badgeSharpArt.fixture.json.
export const SHARP_ORIGIN = 'https://cdn.streamnook.app/twitch-badges/';
/** Two 64-hex names joined by one dot. */
export const SHARP_BASE_RE = /^[0-9a-f]{64}\.[0-9a-f]{64}$/;

export interface SharpUrls {
  large: string;
  small: string;
}

export function sharpUrls(base: unknown): SharpUrls | null {
  if (typeof base !== 'string' || !SHARP_BASE_RE.test(base)) return null;
  const [l, s] = base.split('.');
  return { large: `${SHARP_ORIGIN}${l}.png`, small: `${SHARP_ORIGIN}${s}.webp` };
}
