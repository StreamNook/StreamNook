// Whether a chat link is a picture on one of StreamNook's upload hosts, for
// the OBS overlay's "Uploaded images" switch. TypeScript on purpose: the
// hosted overlay runs in OBS with no Rust, and the builder preview must draw
// exactly what OBS draws, so both use this same function. Kept to the upload
// hosts so a viewer cannot put an arbitrary picture on stream.
//
// The app and streamnook.app keep byte-identical copies of this file.

const HOSTS = ['nuuls.com', 'kappa.lol', 'imgdrip.com', 'catbox.moe', 'uguu.se'];
const EXT = /\.(jpe?g|png|gif|webp|avif|bmp)$/i;

export function isUploadedImage(url: string): boolean {
  try {
    const u = new URL(url);
    if (u.protocol !== 'https:') return false;
    const host = u.hostname.toLowerCase();
    return HOSTS.some((h) => host === h || host.endsWith(`.${h}`)) && EXT.test(u.pathname);
  } catch {
    return false;
  }
}

// Who counts as trusted for "Uploaded images: From trusted chatters": the
// channel's own roles and paying supporters on any platform, by the badge
// names the chat parsers give them.
// - Twitch: broadcaster, lead_moderator, moderator, vip, subscriber, founder.
// - Kick: broadcaster, moderator, vip, og (a role the channel hands out, like
//   VIP), subscriber, founder.
// - YouTube: the owner arrives as broadcaster and a member as subscriber.
//   owner and member stay in the list for any source that names them plainly.
// TikTok badges carry no role name, so TikTok chatters only show pictures
// under "Everyone".
const TRUSTED_BADGES = new Set([
  'broadcaster',
  'lead_moderator',
  'moderator',
  'vip',
  'og',
  'subscriber',
  'founder',
  'owner',
  'member',
]);

export function isTrustedSender(badges: { name: string }[] | undefined): boolean {
  return (badges ?? []).some((b) => TRUSTED_BADGES.has((b.name ?? '').toLowerCase()));
}
