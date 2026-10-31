// Where a tap or click on a chat emote goes, kept pure so it runs in tests. The provider rule is
// the one Rust's native row view uses (chat_row_view.rs `emote_source`); both are tested on
// emoteRefCases.json.
import { canCredit } from '../services/emoteCredit';
import type { EmoteCardTarget } from '../stores/emoteCardStore';

export type EmoteSource = '7tv' | 'twitch' | 'bttv' | 'ffz' | 'kick';

/** Which service an emote image comes from, by its URL and id. BetterTTV ids are 24 hex like
 *  7TV's legacy ones, so the other CDNs are ruled out first. */
export function emoteRef(url: string, id: string | null | undefined): EmoteSource | null {
  if (id) {
    const seventv =
      url.includes('7tv') ||
      ((id.length === 24 || id.length === 26) &&
        !url.includes('jtvnw.net') &&
        !url.includes('frankerfacez') &&
        !url.includes('betterttv'));
    if (seventv) return '7tv';
  }
  if (url.includes('jtvnw.net')) return 'twitch';
  if (url.includes('betterttv')) return 'bttv';
  if (url.includes('frankerfacez')) return 'ffz';
  // Kick's own emotes: the same `kick-<id>` file the emote picker and Rust's prefetch keep.
  if (url.includes('files.kick.com')) return 'kick';
  return null;
}

/** Desktop: a 7TV emote opens the 7TV spotlight where this window has it; anything else the core
 *  can credit opens the emote card; Kick and YouTube emotes have nothing to show. */
export function emoteClickRoute(source: EmoteSource | null, spotlightHere: boolean): 'spotlight' | 'card' | 'none' {
  if (!source) return 'none';
  if (source === '7tv' && spotlightHere) return 'spotlight';
  return canCredit(source) ? 'card' : 'none';
}

/** One image of a row's emote, or of a stack of them (a base and the zero-width ones over it). */
export interface StackMember {
  emoteId?: string;
  emoteUrl?: string;
  content?: string;
}

/** The phone's sheet for a tapped stack: every member the core can credit, in stack order (the
 *  first is the one the sheet opens on). Empty when none can be credited. */
export function stackTargets(
  members: readonly StackMember[],
  room: { login?: string | null; id?: string | null } | null | undefined,
): EmoteCardTarget[] {
  const out: EmoteCardTarget[] = [];
  for (const m of members) {
    if (!m.emoteId) continue;
    const url = m.emoteUrl || `https://static-cdn.jtvnw.net/emoticons/v2/${m.emoteId}/default/dark/2.0`;
    const source = emoteRef(url, m.emoteId);
    if (!source || !canCredit(source)) continue;
    out.push({
      provider: source,
      id: m.emoteId,
      name: m.content ?? '',
      url,
      channelLogin: room?.login ?? null,
      channelId: room?.id ?? null,
    });
  }
  return out;
}
