// The phone's Following list: Twitch follows (the home snapshot) plus live Kick
// follows (followsStore). Deduped on streamKey, which is safe here because Kick
// has no video-id/channel-id split: one live row per channel, keyed `kick:slug`,
// so a Kick channel never collides with the Twitch login of the same name.
import type { TwitchStream } from '../types';
import type { ProviderStreamRow } from '../stores/followsStore';
import { streamKey } from '../utils/streamProvider';

/** Platforms the phone can watch today. YouTube and TikTok rows stay out. */
const PHONE_PROVIDERS: ReadonlySet<string> = new Set(['kick']);

export function mergeFollowedLive(
  twitch: TwitchStream[],
  liveByKey: Record<string, ProviderStreamRow>,
): TwitchStream[] {
  const others = Object.values(liveByKey).filter(
    (r) => r.is_live !== false && PHONE_PROVIDERS.has(r.provider),
  );
  if (others.length === 0) return twitch;
  const seen = new Set(twitch.map(streamKey));
  const out = [...twitch];
  for (const r of others) {
    const k = streamKey(r);
    if (!seen.has(k)) {
      seen.add(k);
      out.push(r);
    }
  }
  // Twitch's followed list already arrives by viewers, so this keeps its order
  // and slots each Kick channel in where its audience puts it.
  return out.sort((a, b) => (b.viewer_count ?? 0) - (a.viewer_count ?? 0));
}
