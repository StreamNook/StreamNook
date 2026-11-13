// Whether a live channel in the drop picker can earn the campaign right now.
//
// Open campaigns are fetched by category, so every channel listed can earn.
// For an allow-listed campaign two answers count, either one enough:
//  - Twitch names the campaign on that channel (`viewerDropCampaigns`). That is
//    what finds a creator campaign filed under one category that credits on
//    the channel whatever it is streaming (Ironmouse's subathon).
//  - The channel is on the allow-list and live in the campaign's category,
//    Twitch's usual rule. `viewerDropCampaigns` answers per viewer and lists
//    only some of a channel's campaigns (QTCinderella's Golden Frog Badge was
//    missing from it while she was live in The Streamer Awards), so it can
//    never be the only gate.

export interface EarnableChannel {
  userId: string;
  currentGame?: string;
  currentGameId?: string;
}

export interface EarnableCampaign {
  isAclBased: boolean;
  /** Channels Twitch says offer the campaign; null when that lookup failed. */
  eligibleIds: ReadonlySet<string> | null;
  gameId?: string;
  gameName?: string;
  /** Every category the drop counts in, when it counts in more than its game. */
  categoryIds?: readonly string[];
}

export function canEarnHere(c: EarnableChannel, campaign: EarnableCampaign): boolean {
  if (!campaign.isAclBased) return true;
  if (campaign.eligibleIds?.has(c.userId)) return true;
  if (c.currentGameId) {
    if (campaign.gameId && c.currentGameId === campaign.gameId) return true;
    if (campaign.categoryIds?.includes(c.currentGameId)) return true;
  }
  return (
    !!c.currentGame &&
    !!campaign.gameName &&
    c.currentGame.toLowerCase() === campaign.gameName.toLowerCase()
  );
}
