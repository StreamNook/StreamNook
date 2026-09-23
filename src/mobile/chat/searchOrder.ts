// Order for the add-chat search: live channels first, biggest audience first,
// then everyone offline in the order the platforms returned them.
//
// Twitch's channel search ranks by name relevance and does not keep live
// channels on top, so the same query could lead with a live channel one time and
// bury it under offline lookalikes the next. Live rooms are what someone is most
// likely opening, so they lead regardless of platform.
import type { TwitchStream } from '../../types';

export function orderSearchResults(rows: TwitchStream[]): TwitchStream[] {
  const live: TwitchStream[] = [];
  const offline: TwitchStream[] = [];
  for (const r of rows) (r.is_live ? live : offline).push(r);
  live.sort((a, b) => (b.viewer_count ?? 0) - (a.viewer_count ?? 0));
  return [...live, ...offline];
}
