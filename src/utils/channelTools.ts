// Which sections of the chat box's Tools menu (components/chat/ChannelToolsMenu)
// apply in a channel. The Tools button shows only when at least one does, so
// the row never carries a button that opens an empty menu.

export interface ChannelToolContext {
  isTwitch: boolean;
  isModerator: boolean;
  isBroadcaster: boolean;
  isVodReplay: boolean;
  /** A drops campaign runs for this game and the drops plugin is installed. */
  hasDropsCampaign: boolean;
}

export interface ChannelToolSections {
  /** Chat modes, moderation and your mod settings: every row is a Helix call. */
  moderate: boolean;
  /** The stream manager rows (stream info, polls, predictions, raid, ads,
   *  rewards queue). Moderators get them too: Rust offers each role only what
   *  Twitch lets it do, through Twitch's own GQL where Helix is broadcaster-only. */
  polls: boolean;
  drops: boolean;
  any: boolean;
}

export function channelToolSections(ctx: ChannelToolContext): ChannelToolSections {
  const moderate = ctx.isTwitch && ctx.isModerator;
  const polls = ctx.isTwitch && (ctx.isBroadcaster || ctx.isModerator) && !ctx.isVodReplay;
  const drops = ctx.hasDropsCampaign;
  return { moderate, polls, drops, any: moderate || polls || drops };
}
