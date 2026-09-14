// Whether a drop campaign can actually be earned on a given channel.
//
// One predicate, shared, because the two surfaces that answer this question
// used to disagree: the stream card showed a drops icon on any channel in a
// drops-enabled category, while the in-player progress bar had already been
// taught that a restricted campaign only credits on its own channels. So a card
// promised drops and opening it delivered nothing.
//
// The rule has now been got wrong in both directions, so it is worth stating
// once and depending on it everywhere. Twitch credits watch-time against a
// campaign only while the channel is streaming that campaign's game, AND, when
// the campaign names channels, only on one of those channels. Category and
// allow-list are both requirements, never alternatives. This function answers
// the second half; callers still have to match the category.
import type { DropCampaign } from '../types';

export function campaignEarnableOn(
  campaign: Pick<DropCampaign, 'allowed_channels' | 'is_acl_based'>,
  channelLogin: string | null | undefined,
): boolean {
  const allow = campaign.allowed_channels || [];
  const restricted = campaign.is_acl_based || allow.length > 0;
  // Category-wide: any channel streaming the game earns it.
  if (!restricted) return true;

  const login = (channelLogin || '').toLowerCase();
  // Restricted and we do not know who we are looking at, so we cannot claim it
  // is earnable. Also covers a campaign flagged ACL that parsed to zero
  // channels, which the backend itself calls unfarmable: no channel could
  // advance it, so there is nothing honest to promise.
  if (!login) return false;
  return allow.some((c) => (c.name || '').toLowerCase() === login);
}
