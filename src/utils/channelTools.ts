// Which sections of the chat box's Tools menu (components/chat/ChannelToolsMenu)
// apply in a channel. The Tools button shows only when at least one does, so
// the row never carries a button that opens an empty menu.
import type { Manages } from '../components/chat/managerStore';

export interface ChannelToolContext {
  isTwitch: boolean;
  isModerator: boolean;
  isVodReplay: boolean;
  /** Whether you manage this channel (`managesHere`): a moderator or
   *  broadcaster badge, or Rust's view listing pages (an editor). */
  manages: Manages;
  /** A drops campaign runs for this game and the drops plugin is installed. */
  hasDropsCampaign: boolean;
}

export interface ChannelToolSections {
  /** Chat modes, moderation and your mod settings: every row is a Helix call. */
  moderate: boolean;
  /** The stream manager rows (stream info, polls, predictions, raid, ads,
   *  rewards queue), for the broadcaster, moderators and editors: Rust offers
   *  each role only what Twitch lets it do. Not while it is still unknown
   *  whether you manage here, nor on a replay's chat. */
  polls: boolean;
  drops: boolean;
  any: boolean;
}

export function channelToolSections(ctx: ChannelToolContext): ChannelToolSections {
  const moderate = ctx.isTwitch && ctx.isModerator;
  const polls = ctx.isTwitch && ctx.manages === 'yes' && !ctx.isVodReplay;
  const drops = ctx.hasDropsCampaign;
  return { moderate, polls, drops, any: moderate || polls || drops };
}
