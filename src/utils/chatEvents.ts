// Chat events as one vocabulary across platforms: which category a notice
// belongs to, whether the viewer has hidden that category for that platform,
// and the values a custom wording template can draw on.
//
// The category map is the same one the OBS overlay uses, kept here so the chat
// row, the chat list and the settings page all agree on what a "gift" is.
import type { EventCategory, EventTemplateContext } from '../components/overlay/overlayConfig';

export type { EventCategory };

const CATEGORY_OF: Record<string, EventCategory> = {
  sub: 'subscription', resub: 'subscription', primepaidupgrade: 'subscription',
  giftpaidupgrade: 'subscription', anongiftpaidupgrade: 'subscription',
  standardpayforward: 'subscription', communitypayforward: 'subscription',
  membership: 'subscription', sharedchatnotice: 'subscription',
  subgift: 'gift', submysterygift: 'gift', anonsubgift: 'gift', anonsubmysterygift: 'gift',
  membergift: 'gift', giftedsub: 'gift', tiktok_gift: 'gift', kick_gift: 'gift', kick_gifted: 'gift',
  raid: 'raid', unraid: 'raid',
  announcement: 'announcement', ritual: 'announcement',
  viewermilestone: 'milestone', watchstreak: 'milestone', bitsbadgetier: 'milestone',
  charitydonation: 'cheer', cheer: 'cheer', bits: 'cheer', superchat: 'cheer', superticker: 'cheer', supersticker: 'cheer',
  tiktok_follow: 'follow', tiktok_share: 'follow', follow: 'follow', kick_follow: 'follow',
};

/** The category of a notice by its msg-id / msg_type, or null for a plain message. */
export function categoryOf(msgType: string | undefined | null): EventCategory | null {
  return msgType ? (CATEGORY_OF[msgType] ?? null) : null;
}

/** The shape of a message as the chat list sees it; only what this needs. */
export interface EventLikeMessage {
  provider?: string;
  metadata?: { msg_type?: string; bits_amount?: number };
  tags?: Map<string, string> | Record<string, string>;
}

const tagOf = (tags: EventLikeMessage['tags'], key: string): string | undefined =>
  !tags ? undefined : tags instanceof Map ? tags.get(key) : tags[key];

/**
 * "provider:category" for an event row, or null for an ordinary message. A
 * Twitch cheer is an ordinary message carrying a bit count, so it is promoted
 * here the way the overlay does it.
 */
export function eventKeyOf(message: EventLikeMessage): string | null {
  const provider = message.provider ?? 'twitch';
  const msgType = message.metadata?.msg_type ?? tagOf(message.tags, 'msg-id');
  let category = categoryOf(msgType);
  if (!category && provider === 'twitch') {
    const bits = message.metadata?.bits_amount ?? Number(tagOf(message.tags, 'bits') ?? 0);
    if (bits > 0) category = 'cheer';
  }
  return category ? `${provider}:${category}` : null;
}

export function isHiddenEvent(message: EventLikeMessage, hidden: readonly string[] | undefined): boolean {
  if (!hidden || hidden.length === 0) return false;
  const key = eventKeyOf(message);
  return key !== null && hidden.includes(key);
}

const SUB_PLAN_LABELS: Record<string, string> = {
  Prime: 'Prime',
  '1000': 'Tier 1',
  '2000': 'Tier 2',
  '3000': 'Tier 3',
};

const count = (v: string | undefined): number | undefined => {
  if (!v) return undefined;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n > 0 ? n : undefined;
};
const text = (v: string | undefined): string | undefined => (v && v.trim() ? v.trim() : undefined);

/**
 * The values a wording template can use, read off a notice's IRC tags. Mirrors
 * the overlay's context so a template written for one works in the other.
 */
export function chatEventTemplateContext(
  tags: Map<string, string>,
  category: EventCategory,
  extra: { username?: string; displayName?: string; provider?: string; bits?: number; channel?: string; time?: string; defaultText?: string },
): EventTemplateContext {
  const plan = tags.get('msg-param-sub-plan');
  const months = count(tags.get('msg-param-cumulative-months')) ?? count(tags.get('msg-param-months'));
  const tierDigit = plan && /^[123]000$/.test(plan) ? Number.parseInt(plan.charAt(0), 10) : undefined;
  const platform = ({ twitch: 'Twitch', kick: 'Kick', youtube: 'YouTube', tiktok: 'TikTok' } as Record<string, string>)[
    extra.provider ?? 'twitch'
  ] ?? 'Twitch';
  return {
    username: extra.displayName || extra.username || undefined,
    userLogin: extra.username || undefined,
    tier: plan ? (SUB_PLAN_LABELS[plan] ?? plan) : text(tags.get('msg-param-sub-plan-name')),
    tierNumber: tierDigit,
    planName: text(tags.get('msg-param-sub-plan-name')),
    months,
    years: months && months >= 12 ? Math.floor(months / 12) : undefined,
    streak: category === 'milestone' ? count(tags.get('msg-param-value')) : count(tags.get('msg-param-streak-months')),
    giftMonths: count(tags.get('msg-param-gift-months')),
    multimonth: count(tags.get('msg-param-multimonth-duration')),
    priorGifter: text(tags.get('msg-param-prior-gifter-display-name')),
    recipient: text(tags.get('msg-param-recipient-display-name')),
    recipientLogin: text(tags.get('msg-param-recipient-user-name')),
    count: count(tags.get('msg-param-mass-gift-count')),
    gifterTotal: count(tags.get('msg-param-sender-count')),
    bits: extra.bits && extra.bits > 0 ? extra.bits : undefined,
    charity: text(tags.get('msg-param-charity-name')),
    amount: undefined,
    viewers: count(tags.get('msg-param-viewerCount')),
    points: count(tags.get('msg-param-copoReward')),
    channel: extra.channel,
    platform,
    time: extra.time,
    default: extra.defaultText,
  };
}
