// Overlay style config — the single source of truth for how the OBS chat
// overlay looks. Set in the in-app builder, saved per overlay, and read by both
// the builder preview and (later) the hosted overlay page, so the two can never
// drift. Phase 1 is builder + preview only; nothing here talks to a backend.

import type { ProviderId } from '../../types/providers';

// Overlay event categories — each stream event reflects its actual type (a watch
// streak is a Milestone, never a Subscription). Kept here (not in OverlayChat) so
// both the renderer and the builder's event filter share one source of truth.
export type EventCategory = 'subscription' | 'gift' | 'raid' | 'cheer' | 'milestone' | 'redemption' | 'follow' | 'announcement';

export const EVENT_CATEGORIES: { id: EventCategory; label: string }[] = [
  { id: 'subscription', label: 'Subscriptions' },
  { id: 'gift', label: 'Gifts' },
  { id: 'raid', label: 'Raids' },
  { id: 'cheer', label: 'Bits & Super Chats' },
  { id: 'milestone', label: 'Milestones' },
  { id: 'redemption', label: 'Channel points' },
  { id: 'follow', label: 'Follows' },
  { id: 'announcement', label: 'Announcements' },
];

/** How a bits cheer renders. A cheer arrives as an ordinary chat message carrying a
 *  bit count, so 'message' is Twitch's own layout; 'event' promotes it to the same
 *  card subs and raids get. */
export type CheerDisplay = 'message' | 'event';

export const CHEER_DISPLAYS: { value: CheerDisplay; label: string }[] = [
  { value: 'message', label: 'Message' },
  { value: 'event', label: 'Event card' },
];

/** How a reply renders. 'full' draws the "Replying to @name: body" context line
 *  above the message; 'mention' drops the context line and prefixes the body with
 *  the parent's "@name" the way Twitch chat did before threading; 'off' shows the
 *  message alone with no sign it was a reply. */
export type ReplyStyle = 'full' | 'mention' | 'off';

export const REPLY_STYLES: { value: ReplyStyle; label: string }[] = [
  { value: 'full', label: 'Context line' },
  { value: 'mention', label: '@username' },
  { value: 'off', label: 'Off' },
];

/** How a URL in a message body renders. 'accent' colors it (linkColor) the way chat
 *  clients do; 'plain' leaves it in the body text color so it reads as ordinary text. */
export type LinkStyle = 'accent' | 'plain';

export const LINK_STYLES: { value: LinkStyle; label: string }[] = [
  { value: 'accent', label: 'Accent' },
  { value: 'plain', label: 'Body text' },
];

/** Default link accent when linkColor is left empty. */
export const DEFAULT_LINK_COLOR = '#8ab4ff';

export type SourceTagMode = 'none' | 'dot' | 'label' | 'icon';
export type OverlayBackground = 'transparent' | 'solid';
export type OverlayDirection = 'newBottom' | 'newTop';
export type OverlayEntrance = 'none' | 'fade' | 'slide' | 'drift' | 'rise' | 'pop' | 'stamp';

export const OVERLAY_ENTRANCES: { value: OverlayEntrance; label: string }[] = [
  { value: 'none', label: 'None' },
  { value: 'fade', label: 'Fade' },
  { value: 'slide', label: 'Slide' },
  { value: 'drift', label: 'Drift' },
  { value: 'rise', label: 'Rise' },
  { value: 'pop', label: 'Pop' },
  { value: 'stamp', label: 'Stamp' },
];
export type OverlayTextAlign = 'left' | 'center' | 'right';

export const OVERLAY_TEXT_ALIGNS: { value: OverlayTextAlign; label: string }[] = [
  { value: 'left', label: 'Left' },
  { value: 'center', label: 'Center' },
  { value: 'right', label: 'Right' },
];

/** Text weights the control offers. Values are strings because SegmentedSelect is
 *  generic over `T extends string`; the setter parses back to a number. Capped at
 *  700 because that is the heaviest real face every preset family ships — past it
 *  the browser fakes the weight instead of loading one. */
export const OVERLAY_TEXT_WEIGHTS: { value: string; label: string }[] = [
  { value: '300', label: 'Light' },
  { value: '400', label: 'Regular' },
  { value: '500', label: 'Medium' },
  { value: '600', label: 'Semibold' },
  { value: '700', label: 'Bold' },
];

/** Where a gigantified emote lands. The three block modes pluck it out of the
 *  message onto its own line below; 'inline' leaves it where the sender typed it,
 *  at 4x — so an emote-only message shows it right after the name, like a normal
 *  message body. */
export type GiantEmoteAlign = 'left' | 'center' | 'right' | 'inline';

export const GIANT_EMOTE_ALIGNS: { value: GiantEmoteAlign; label: string }[] = [
  { value: 'left', label: 'Left' },
  { value: 'center', label: 'Center' },
  { value: 'right', label: 'Right' },
  { value: 'inline', label: 'Inline' },
];

export type EmojiStyle = 'system' | 'apple' | 'google' | 'twitter' | 'facebook';
export type FirstTimeStyle = 'off' | 'twitch' | 'streamnook';
export type BubbleShape = 'rounded' | 'pill' | 'speech';

export const BUBBLE_SHAPES: { value: BubbleShape; label: string }[] = [
  { value: 'rounded', label: 'Rounded' },
  { value: 'pill', label: 'Pill' },
  { value: 'speech', label: 'Speech' },
];
/** Border accent animations (first-time highlight + Outline events). All ride the
 *  border only, never the fill: 'sheen' sweeps a glint across it, 'pulse'
 *  breathes it brighter, 'chase' sends a spark around the ring. */
export type OverlayAnimation = 'none' | 'sheen' | 'pulse' | 'chase';

export const OVERLAY_ANIMATIONS: { value: OverlayAnimation; label: string }[] = [
  { value: 'none', label: 'None' },
  { value: 'sheen', label: 'Sheen' },
  { value: 'pulse', label: 'Pulse' },
  { value: 'chase', label: 'Chase' },
];

export interface OverlayStyle {
  /** Overlay canvas size in px (the Browser Source dimensions in OBS). Taller =
   *  more chat visible at once. */
  width: number;
  height: number;
  /** Platforms whose messages the overlay shows. */
  sources: ProviderId[];
  /** Whether/how to mark which platform each message came from. */
  sourceTag: SourceTagMode;

  /** Font family (a CSS font-family string, chosen from FONT_OPTIONS). */
  fontFamily: string;
  /** Base message font size in px. Emotes/badges scale off this. */
  fontSize: number;
  /** Line height multiplier for wrapped message text. */
  lineHeight: number;
  /** Vertical gap between messages in px. */
  messageGap: number;

  /** Inline emote size multiplier (1 = default 2em). */
  emoteScale: number;
  /** Badge size multiplier (1 = default, ~1.35em tall). */
  badgeScale: number;
  showBadges: boolean;
  showTimestamps: boolean;

  /** Color for plain message body text. Usernames keep their own color/paint. */
  bodyTextColor: string;
  /** Drop a shadow so the overlay stays legible over any scene. The shadowOn*
   *  switches pick what it sits behind; color, size, strength and style are shared. */
  textShadow: boolean;
  /** Text shadow color. Used when textShadow is on. */
  textShadowColor: string;
  /** Text shadow blur in px at the configured font size (0 = no shadow). */
  textShadowSize: number;
  /** Text shadow strength, 0 to 1. */
  textShadowOpacity: number;
  /** The shadow sits behind message and event text. */
  shadowOnText: boolean;
  /** The shadow sits behind usernames and their colon, 7TV paints included. */
  shadowOnNames: boolean;
  /** The shadow sits behind badges (platform, add-on and StreamNook), shaped to
   *  the badge art. */
  shadowOnBadges: boolean;
  /** A heavier, darker shadow behind text: a tight outline plus the stacked
   *  layers a 7TV paint's shadow casts. */
  shadowDarkText: boolean;
  /** A heavier, darker shadow behind names. */
  shadowDarkNames: boolean;
  /** A heavier, darker shadow behind badges. */
  shadowDarkBadges: boolean;

  /** Horizontal justification of message and event text. */
  textAlign: OverlayTextAlign;
  /** Text weight, 300-700. Names and the other deliberately-bold bits set their own
   *  weight and are unaffected. Capped at 700: the heaviest real face every preset
   *  family ships. */
  fontWeight: number;
  /** Slant the message body. /me actions render italic either way. */
  textItalic: boolean;
  /** Strike a line through message body text. */
  textStrikethrough: boolean;

  background: OverlayBackground;
  /** Used when background === 'solid'. */
  backgroundColor: string;
  /** 0–1, applied to the solid background only. */
  backgroundOpacity: number;

  /** How stream events render. Every style shows the sender decorated (badges +
   *  paint name) + the event action. 'plain' keeps a subtle per-source tint,
   *  'outline' draws a thin ring in the source's color, and 'streamnook' adds
   *  the app's signature multi-color gradient wash. */
  eventStyle: 'plain' | 'streamnook' | 'outline';
  /** Outline events only: a nearly-transparent color-matched tint inside the
   *  ring, so the event reads highlighted instead of just bordered. */
  eventFill: boolean;
  /** Outline events only: border accent when the event lands. See
   *  OverlayAnimation. Plays once on arrival unless eventAnimateRepeat. */
  eventAnimation: OverlayAnimation;
  /** Replay the event animation every ~5 seconds while the event is on screen. */
  eventAnimateRepeat: boolean;
  /** How a bits cheer renders: inline as a normal message, or as an event card. */
  cheerDisplay: CheerDisplay;
  /** Show a row when the channel's 7TV emote set changes (an emote added,
   *  removed or renamed), styled like the other events. */
  seventvEmoteChanges: boolean;
  /** Hide messages from known bot accounts / users carrying a bot badge. */
  hideBots: boolean;
  /** Legacy global event hides (e.g. 'raid', 'cheer'), applied to every platform.
   *  Superseded by the per-source hiddenProviderEvents below: clampOverlayStyle
   *  folds anything here into per-source keys and clears this, so new configs
   *  leave it empty. Kept in the type only so old saved configs still parse. */
  hiddenEvents: string[];
  /** The per-source event filter: which event categories are hidden, keyed
   *  `provider:category` (e.g. 'tiktok:follow'). Each platform is filtered on its
   *  own, so a Twitch raid and a Kick raid toggle independently. Empty = show all. */
  hiddenProviderEvents: string[];
  /** Target ISO currency to convert YouTube Super Chats into ('' = show as sent). */
  superchatCurrency: string;
  /** Per-source username blocklist, keyed `${provider}:${channel}` → usernames to
   *  hide (case-insensitive, matched against username OR display name). For bots the
   *  auto-hider misses. Matching is effectively per-platform (a name blocked on any
   *  source of a platform is hidden for that platform). */
  blockedUsers: Record<string, string[]>;
  /** Hide chat messages whose body starts with a command prefix (e.g. "!title").
   *  Only applies to normal messages, never to events. */
  hideCommands: boolean;
  /** Command filters applied when hideCommands is on. Each entry has an explicit
   *  mode: 'prefix' hides every message starting with `value` (e.g. '!' or '#'),
   *  'exact' hides only messages whose first word equals `value` (e.g. '!title'). */
  commandFilters: { value: string; mode: 'prefix' | 'exact' }[];
  /** Show 7TV paints on usernames. */
  showPaints: boolean;
  /** Show third-party badges (7TV, FFZ, Chatterino, and similar). Native platform
   *  badges are controlled separately by showBadges. */
  showThirdPartyBadges: boolean;
  /** Show StreamNook atmosphere backgrounds behind a member's chat row. */
  showAtmospheres: boolean;
  /** Third-party badge providers to hide, by id (e.g. 'ffz', 'chatterino', 'bttv',
   *  '7tv'). Each provider toggles independently, on top of the global
   *  showThirdPartyBadges master. See THIRD_PARTY_BADGE_PROVIDERS. */
  hiddenBadgeProviders: string[];
  /** Twitch badge categories to hide, by id (e.g. 'predictions', 'roles'), on top
   *  of the showBadges master. See TWITCH_BADGE_CATEGORIES. */
  hiddenTwitchBadgeCategories: string[];
  /** Draw a channel's own FrankerFaceZ moderator and VIP badge art instead of
   *  Twitch's. Off shows Twitch's standard moderator and VIP badges, since a
   *  channel's custom art can be anything. */
  ffzCustomRoleBadges: boolean;
  /** How unicode emoji render. 'system' uses the OS/font emoji (varies by machine);
   *  the vendor styles re-render EVERY emoji from every platform as that one style's
   *  images, so a merged overlay looks consistent regardless of platform or OS. */
  emojiStyle: EmojiStyle;
  /** Show chatter profile pictures (YouTube and TikTok carry avatars). */
  showAvatars: boolean;
  /** Show the @ some platforms put in front of usernames (YouTube handles are
   *  "@name"). Off strips the leading @ wherever a name renders. */
  showAtSign: boolean;
  /** Walk a chatter's own name color lighter (or darker) until it reads against
   *  the overlay's backdrop. Hue and saturation stay; 7TV paints are untouched. */
  readableNameColors: boolean;
  /** Legacy reply toggle. Superseded by replyStyle: clampOverlayStyle migrates
   *  `false` here into replyStyle 'off'. Kept in the type only so old saved
   *  configs still parse. */
  showReplies: boolean;
  /** How a reply renders — context line, bare "@name" prefix, or nothing. */
  replyStyle: ReplyStyle;
  /** Whether a URL gets its own accent color or stays in the body text color. */
  linkStyle: LinkStyle;
  /** Accent color for links when linkStyle is 'accent'. '' = DEFAULT_LINK_COLOR. */
  linkColor: string;
  /** Underline links. Independent of linkStyle, so plain-colored links can still
   *  be underlined (and accented ones left clean). */
  linkUnderline: boolean;
  /** Render 7TV personal emotes — a subscriber's own set, which works in every
   *  channel. On by default (it matches the 7TV extension), but a streamer whose
   *  channel has no 7TV emotes still sees them from chatters who do, which reads
   *  as emotes appearing from nowhere. Off renders the emote's name as text. */
  showPersonalEmotes: boolean;
  /** Custom wording per event category, replacing the platform's system message.
   *  Values are `{token}` templates; see renderEventTemplate. Absent/empty for a
   *  category = keep the platform's own wording. */
  eventTemplates: Partial<Record<EventCategory, string>>;
  /** How a chatter's first-ever message in the channel is marked. 'twitch' draws
   *  the outline + label Twitch's own chat uses; 'streamnook' uses the app chat's
   *  purple highlight (gradient wash + left border + label). Twitch sends the
   *  signal; other platforms don't, so it only ever fires on Twitch messages. */
  firstTimeStyle: FirstTimeStyle;
  /** Twitch style only: a nearly-transparent color-matched tint inside the ring,
   *  so the row reads highlighted instead of just bordered. (The StreamNook
   *  style's gradient wash is its own fill.) */
  firstTimeFill: boolean;
  /** Border accent when a first-time chatter's message lands (around the ring
   *  for the Twitch style, down the left bar for StreamNook). See
   *  OverlayAnimation. Plays once on arrival unless firstTimeAnimateRepeat. */
  firstTimeAnimation: OverlayAnimation;
  /** Replay the animation every ~5 seconds while the message is on screen,
   *  instead of once on arrival. Costs a little OBS paint work while a
   *  first-time message is visible (an idle overlay otherwise paints nothing). */
  firstTimeAnimateRepeat: boolean;
  /** Whether new messages appear at the bottom (chat-style) or the top. */
  direction: OverlayDirection;
  /** Entrance animation for incoming messages. */
  entrance: OverlayEntrance;
  /** Draw each chat message in its own bubble that hugs the text, instead of
   *  bare text over the scene. A member's atmosphere wash replaces the bubble
   *  on their rows; events keep their own event styling. */
  bubble: boolean;
  /** Bubble silhouette: 'rounded' uses bubbleRadius on every corner, 'pill'
   *  fully rounds the ends, 'speech' keeps rounded corners but tucks the
   *  bottom-left one in (a messenger-style tail corner). */
  bubbleShape: BubbleShape;
  /** Corner radius in px for the rounded/speech shapes (pill ignores it). */
  bubbleRadius: number;
  /** Bubble background color. */
  bubbleColor: string;
  /** Bubble background opacity, 0 to 1. */
  bubbleOpacity: number;
  /** Custom accent for the first-time chatter highlight (outline, fill, bar,
   *  wash, and label all follow it). '' = the style's own default: Twitch pink
   *  or StreamNook purple. */
  firstTimeColor: string;
  /** Outline events only: one fixed ring color for every event. '' = each
   *  event uses its source platform's color. */
  eventOutlineColor: string;
  /** Hide chat messages containing any of these words/phrases (case-insensitive
   *  substring). Never hides events. */
  hidePhrases: string[];
  /** Remove a message this many seconds after it appeared on the overlay
   *  (0 = keep it until it scrolls off). */
  maxMessageAgeSec: number;
  /** Clamp each chat message to this many lines, ending in an ellipsis
   *  (0 = no limit). */
  maxMessageLines: number;
  /** Restore the last on-screen messages after an OBS/browser source reload.
   *  Off (default) = the overlay comes back cleared on reload / stream start. */
  restoreOnReload: boolean;
  /** Fill the overlay with the Twitch channel's recent chat when it starts. */
  loadRecentChat: boolean;
  /** Let the broadcaster and moderators type !refreshoverlay or !clearoverlay in
   *  Twitch chat to reload or clear the overlay without touching OBS. */
  modCommands: boolean;
  /** Render the last emote of a "Gigantify an Emote" power-up message at 4x
   *  below the message, like Twitch does. Off shows it inline at normal size. */
  giantEmotes: boolean;
  /** Where the gigantified emote lands. See GiantEmoteAlign. */
  giantEmoteAlign: GiantEmoteAlign;
  /** Twitch chat GIFs (GIPHY-backed, Tier 2/3 subscribers): draw them at the
   *  gigantified size, inline after the name. Off shows the description
   *  Twitch puts in the message text instead. */
  showGifs: boolean;
  /** Chat GIF size multiplier on top of the gigantified size (which already
   *  follows emoteScale). 1 = the size GIFs have always drawn at. */
  gifScale: number;
  /** Pictures linked from StreamNook's upload hosts (uploadedImage.ts) draw in
   *  the message in place of their link. Off (default) keeps every link text. */
  showUploadedImages: boolean;
  /** Uploaded picture height multiplier. 1 = 6em tall at most. */
  uploadedImageScale: number;
  /** Whose uploaded pictures draw: the channel's roles and subscribers
   *  (isTrustedSender), or every chatter. */
  uploadedImagesFrom: 'trusted' | 'everyone';
}

// ---------------------------------------------------------------------------
// Custom event text
// ---------------------------------------------------------------------------
// Each event category can carry a template that replaces the platform's own
// system message ("winters27 subscribed at Tier 1. They've subscribed for 94
// months!") with the streamer's own wording. Tokens are `{name}` and resolve
// from the event's real data, so nothing here invents a value.

/** The values a template can reference. Absent = the event did not carry it, and
 *  renderEventTemplate then falls back to the platform's own message. Everything
 *  here comes off the event; nothing is invented or guessed. */
export interface EventTemplateContext {
  // ── Who ──────────────────────────────────────────────────────────────────
  /** Display name of whoever triggered the event (subscriber, gifter, raider). */
  username?: string;
  /** Their lowercase login, for templates that want an @-less or URL-ish form. */
  userLogin?: string;

  // ── Subscription ─────────────────────────────────────────────────────────
  /** Sub tier as the platform labels it ("Tier 1", "Prime", a YouTube tier name). */
  tier?: string;
  /** Just the digit: 1, 2 or 3. Absent for Prime, which has no tier number. */
  tierNumber?: number;
  /** The channel's own name for the plan, when it set one. */
  planName?: string;
  /** Total months subscribed, cumulative across gaps. */
  months?: number;
  /** Whole years subscribed. Absent under 12 months, so "{years} years" can't say 0. */
  years?: number;
  /** Consecutive months (a sub streak) or days (a watch-streak milestone). */
  streak?: number;
  /** Months covered by a gift. */
  giftMonths?: number;
  /** Months bought up front in a multi-month sub. */
  multimonth?: number;
  /** Who gifted the sub being upgraded or paid forward. */
  priorGifter?: string;

  // ── Gifts ────────────────────────────────────────────────────────────────
  /** Gift recipient's display name. Absent on community gifts (no single target). */
  recipient?: string;
  /** Recipient's lowercase login. */
  recipientLogin?: string;
  /** How many were gifted in a community gift. */
  count?: number;
  /** The gifter's lifetime gift count in this channel. */
  gifterTotal?: number;

  // ── Money ────────────────────────────────────────────────────────────────
  /** Bits cheered / Super Chat amount as sent. */
  bits?: number;
  /** Charity name on a charity donation. */
  charity?: string;
  /** Donation amount, already formatted with its currency symbol. */
  amount?: string;

  // ── Channel points ───────────────────────────────────────────────────────
  /** The reward a viewer spent channel points on, as the channel named it. */
  reward?: string;
  /** What that reward cost, in channel points, with thousands separators. */
  cost?: string;

  // ── Raid ─────────────────────────────────────────────────────────────────
  /** Viewers brought by a raid. */
  viewers?: number;

  // ── Milestones ───────────────────────────────────────────────────────────
  /** Channel points earned from a watch-streak milestone. */
  points?: number;

  // ── Context ──────────────────────────────────────────────────────────────
  /** The channel the event landed in. */
  channel?: string;
  /** Platform label: Twitch, Kick, YouTube, TikTok. */
  platform?: string;
  /** The event's timestamp, formatted the way the overlay formats timestamps. */
  time?: string;
  /** Whatever the platform itself would have said, with the leading name removed
   *  ("subscribed at Tier 1 for 94 months"). Lets a template add to the default
   *  instead of replacing it: "{username} {default} — welcome back!". */
  default?: string;
}

/** One row of the token reference.
 *
 *  `example` is the point of this table. A token is only obvious once you see the
 *  value it stands in for, so the builder shows the example everywhere the token
 *  appears and renders a live preview from these values as you type. Keep them
 *  realistic and mutually consistent — they read as one imaginary event. */
export const EVENT_TEMPLATE_TOKENS: {
  token: keyof EventTemplateContext;
  label: string;
  example: string;
  group: string;
}[] = [
  { token: 'username', label: 'Who triggered it', example: 'Winters27', group: 'Who' },
  { token: 'userLogin', label: 'The same name, all lowercase', example: 'winters27', group: 'Who' },
  { token: 'tier', label: 'Sub tier, worded the way the platform words it', example: 'Tier 1', group: 'Subscription' },
  { token: 'tierNumber', label: 'Just the tier digit. Prime subs have none', example: '1', group: 'Subscription' },
  { token: 'planName', label: "The channel's own name for the sub plan", example: 'Channel Subscription (mathox)', group: 'Subscription' },
  { token: 'months', label: 'Months subscribed in total', example: '94', group: 'Subscription' },
  { token: 'years', label: 'Whole years subscribed. Nothing under 12 months', example: '7', group: 'Subscription' },
  { token: 'streak', label: 'Months in a row subscribed, or days in a watch streak', example: '12', group: 'Subscription' },
  { token: 'giftMonths', label: 'How many months the gift covers', example: '3', group: 'Subscription' },
  { token: 'multimonth', label: 'Months bought in one go', example: '6', group: 'Subscription' },
  { token: 'priorGifter', label: 'Who gifted the sub being continued', example: 'Zo0x_', group: 'Subscription' },
  { token: 'recipient', label: 'Who got the gift', example: 'Mathox', group: 'Gifts' },
  { token: 'recipientLogin', label: 'The same name, all lowercase', example: 'mathox', group: 'Gifts' },
  { token: 'count', label: 'How many subs were gifted at once', example: '50', group: 'Gifts' },
  { token: 'gifterTotal', label: 'How many they have gifted here in total', example: '412', group: 'Gifts' },
  { token: 'bits', label: 'Bits cheered, or the Super Chat amount', example: '1000', group: 'Money' },
  { token: 'charity', label: 'Which charity a donation went to', example: 'Direct Relief', group: 'Money' },
  { token: 'amount', label: 'Donation amount, currency symbol included', example: '$12.34', group: 'Money' },
  { token: 'viewers', label: 'How many viewers the raid brought', example: '237', group: 'Raid' },
  { token: 'points', label: 'Channel points earned from a watch streak', example: '350', group: 'Milestones' },
  { token: 'reward', label: 'The reward they spent points on', example: 'Hydrate', group: 'Channel points' },
  { token: 'cost', label: 'What the reward cost in points', example: '5,000', group: 'Channel points' },
  { token: 'channel', label: 'The channel it happened in', example: 'mathox', group: 'Context' },
  { token: 'platform', label: 'Which platform it came from', example: 'Twitch', group: 'Context' },
  { token: 'time', label: 'When it happened', example: '3:45 PM', group: 'Context' },
  { token: 'default', label: 'What the platform would have said on its own', example: 'subscribed at Tier 1 for 94 months', group: 'Context' },
];

/** The examples as a context, so the builder can render a template live. Every
 *  token resolves here, which is the point: the preview shows the SHAPE of the
 *  sentence. Real events fall back whole when a value is missing. */
export const SAMPLE_TEMPLATE_CONTEXT: EventTemplateContext = Object.fromEntries(
  EVENT_TEMPLATE_TOKENS.map((t) => [t.token, t.example]),
) as EventTemplateContext;

/** {default} is the one token whose real value changes with the event type, so a
 *  single sample would show a sub sentence in the Raids preview. Per-category
 *  samples keep the preview honest about what the streamer would actually get. */
export const CATEGORY_DEFAULT_EXAMPLES: Record<EventCategory, string> = {
  subscription: 'subscribed at Tier 1 for 94 months',
  gift: 'is gifting 50 subs to the community',
  raid: 'is raiding with 237 viewers',
  cheer: 'cheered 1000 bits',
  milestone: 'watched 12 consecutive streams',
  redemption: 'redeemed Hydrate',
  follow: 'followed',
  announcement: 'we go live an hour early tomorrow',
};

/** The sample context for one category: shared examples, with {default} swapped
 *  for the wording that category would really produce. */
export const sampleContextFor = (category: EventCategory): EventTemplateContext => ({
  ...SAMPLE_TEMPLATE_CONTEXT,
  default: CATEGORY_DEFAULT_EXAMPLES[category],
});

/** Tokens every category can resolve, appended to each list below. */
const COMMON_TOKENS: (keyof EventTemplateContext)[] = [
  'username', 'userLogin', 'channel', 'platform', 'time', 'default',
];

/** Which tokens can carry a value for each category, so the builder only offers
 *  tokens that can resolve rather than every token everywhere. */
export const CATEGORY_TEMPLATE_TOKENS: Record<EventCategory, (keyof EventTemplateContext)[]> = {
  subscription: ['months', 'years', 'streak', 'tier', 'tierNumber', 'planName', 'giftMonths', 'multimonth', 'priorGifter', ...COMMON_TOKENS],
  gift: ['recipient', 'recipientLogin', 'count', 'gifterTotal', 'tier', 'tierNumber', 'giftMonths', ...COMMON_TOKENS],
  raid: ['viewers', ...COMMON_TOKENS],
  cheer: ['bits', 'charity', 'amount', ...COMMON_TOKENS],
  milestone: ['streak', 'points', 'months', ...COMMON_TOKENS],
  redemption: ['reward', 'cost', ...COMMON_TOKENS],
  follow: [...COMMON_TOKENS],
  announcement: [...COMMON_TOKENS],
};

/** A starting point per category, shown as the placeholder in the builder. */
export const EVENT_TEMPLATE_EXAMPLES: Record<EventCategory, string> = {
  subscription: '{username} just re-subscribed for {months} months!',
  gift: '{username} gifted {count} subs to the channel!',
  raid: '{username} is raiding with {viewers} viewers!',
  cheer: '{username} cheered {bits} bits!',
  milestone: '{username} has been watching for {streak} in a row!',
  redemption: '{username} spent {cost} points on {reward}',
  follow: 'Welcome in, {username}!',
  announcement: '{username} says: {default}',
};

const TOKEN_RE = /\{([a-zA-Z]+)\}/g;

/**
 * Fill a template from an event's real values.
 *
 * Returns null when the template is empty or references a token this event did
 * not carry — the caller then falls back to the platform's own system message.
 * Falling back whole is deliberate: a partial fill reads as broken ("re-subscribed
 * for  months!"), and an event missing its data is better shown as the platform
 * worded it than as a sentence with a hole in it.
 */
export function renderEventTemplate(template: string, ctx: EventTemplateContext): string | null {
  const t = (template || '').trim();
  if (!t) return null;
  let missing = false;
  const out = t.replace(TOKEN_RE, (whole, name: string) => {
    if (!(name in ctx)) { missing = true; return whole; }
    const v = ctx[name as keyof EventTemplateContext];
    if (v === undefined || v === null || v === '') { missing = true; return whole; }
    return String(v);
  });
  return missing ? null : out;
}

/** Strip templates down to the categories that exist, trimmed, empties dropped. */
export function sanitizeEventTemplates(raw: unknown): Partial<Record<EventCategory, string>> {
  if (!raw || typeof raw !== 'object') return {};
  const valid = new Set(EVENT_CATEGORIES.map((c) => c.id));
  const out: Partial<Record<EventCategory, string>> = {};
  for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
    if (!valid.has(k as EventCategory)) continue;
    const text = String(v ?? '').trim().slice(0, 200);
    if (text) out[k as EventCategory] = text;
  }
  return out;
}

// Which event categories each platform can actually emit — drives the per-platform
// event toggles in the builder so a provider only shows togglable event types it
// produces (Twitch has raids/milestones, TikTok has follows, etc.).
export const PROVIDER_EVENT_CATEGORIES: Partial<Record<ProviderId, EventCategory[]>> = {
  twitch: ['subscription', 'gift', 'cheer', 'raid', 'milestone', 'redemption', 'announcement'],
  kick: ['subscription', 'gift', 'follow', 'raid'],
  youtube: ['subscription', 'gift', 'cheer'],
  tiktok: ['gift', 'cheer', 'follow'],
};

// A category's name on a SPECIFIC platform, where the generic label would name
// another platform's feature: the money category is Bits on Twitch, Super Chats
// on YouTube, coins on TikTok. Fall back to EVENT_CATEGORIES for the rest.
export const PROVIDER_CATEGORY_LABELS: Partial<Record<ProviderId, Partial<Record<EventCategory, string>>>> = {
  twitch: { cheer: 'Bits' },
  youtube: { cheer: 'Super Chats' },
  tiktok: { cheer: 'Coins' },
};

// Badge providers the overlay resolves, each independently toggleable under the
// showThirdPartyBadges master. The `id` matches the `source` tagged on each
// resolved badge (7TV is the separate seventvBadge, StreamNook is the separate
// member-badge slot, everything else arrives in extraBadges with its provider).
export const THIRD_PARTY_BADGE_PROVIDERS: { id: string; label: string }[] = [
  { id: 'streamnook', label: 'StreamNook' },
  { id: '7tv', label: '7TV' },
  { id: 'ffz', label: 'FFZ' },
  { id: 'chatterino', label: 'Chatterino' },
  { id: 'homies', label: 'Homies' },
  { id: 'homies-custom', label: 'Homies custom' },
  { id: 'moltorino', label: 'Moltorino' },
  { id: 'bttv', label: 'BTTV' },
  { id: 'chatsen', label: 'Chatsen' },
  { id: 'chatty', label: 'Chatty' },
  { id: 'dankchat', label: 'DankChat' },
];

// Twitch badge categories a streamer can hide, each listing its Twitch badge SET
// ids. The same table as TWITCH_BADGE_CATEGORY_SETS in the backend's
// models/chat_layout.rs: the hosted overlay page has no backend, so the renderer
// classifies by itself, and twitchBadgeCategories.test.ts fails when the two
// tables differ. 'other' is every remaining set; the broadcaster and staff
// badges sit in no category and follow only the Badges switch.
export const TWITCH_BADGE_CATEGORIES: { id: string; label: string; sets: string[] }[] = [
  { id: 'subscriber', label: 'Subscriber', sets: ['subscriber', 'founder'] },
  { id: 'predictions', label: 'Prediction', sets: ['predictions'] },
  { id: 'gifted-subs', label: 'Gifted subs', sets: ['sub-gifter', 'sub-gift-leader'] },
  { id: 'bits', label: 'Bits', sets: ['bits', 'bits-leader', 'bits-charity', 'anonymous-cheerer'] },
  { id: 'blossom', label: 'Blossom', sets: ['blossom-badge'] },
  { id: 'bloom', label: 'Bloom', sets: ['bloom-badge'] },
  { id: 'roles', label: 'Authority roles', sets: ['lead_moderator', 'moderator', 'vip'] },
  { id: 'other', label: 'Everything else', sets: [] },
];

const TWITCH_BADGE_SETS_NEVER_CATEGORISED = new Set(['broadcaster', 'staff', 'admin', 'global_mod']);
const TWITCH_BADGE_CATEGORY_BY_SET = new Map(
  TWITCH_BADGE_CATEGORIES.flatMap((c) => c.sets.map((set) => [set, c.id] as const)),
);

/** The hide category of one Twitch badge set id, or null for a set no category
 *  covers (the broadcaster and staff badges, or an empty id). */
export function twitchBadgeCategory(setId: string | null | undefined): string | null {
  if (!setId || TWITCH_BADGE_SETS_NEVER_CATEGORISED.has(setId)) return null;
  return TWITCH_BADGE_CATEGORY_BY_SET.get(setId) ?? 'other';
}

// Font choices mirror the app's Theme › Font list so an overlay can match the
// streamer's in-app look. Values are CSS font-family strings.
// Unicode emoji rendering styles. 'system' = the OS/font emoji; the rest are image
// sets served from jsDelivr (emoji-datasource-<style>), sharing one codepoint
// filename convention so every emoji renders in the chosen style.
export const EMOJI_STYLES: { value: EmojiStyle; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'apple', label: 'Apple' },
  { value: 'google', label: 'Google' },
  { value: 'twitter', label: 'Twitter' },
  { value: 'facebook', label: 'Facebook' },
];

export const FONT_OPTIONS: { label: string; value: string }[] = [
  { label: 'Inter', value: "'Inter', system-ui, sans-serif" },
  { label: 'Satoshi', value: "'Satoshi', system-ui, sans-serif" },
  { label: 'Geist', value: "'Geist', system-ui, sans-serif" },
  { label: 'Manrope', value: "'Manrope', system-ui, sans-serif" },
  { label: 'Outfit', value: "'Outfit', system-ui, sans-serif" },
  { label: 'Space Grotesk', value: "'Space Grotesk', system-ui, sans-serif" },
  { label: 'System', value: 'system-ui, sans-serif' },
];

export const DEFAULT_OVERLAY_STYLE: OverlayStyle = {
  width: 400,
  height: 640,
  sources: ['twitch', 'kick', 'youtube', 'tiktok'],
  sourceTag: 'dot',
  fontFamily: FONT_OPTIONS[0].value,
  fontSize: 15,
  lineHeight: 1.4,
  messageGap: 6,
  emoteScale: 1,
  badgeScale: 1,
  showBadges: true,
  showTimestamps: false,
  bodyTextColor: '#ffffff',
  textShadow: true,
  textShadowColor: '#000000',
  textShadowSize: 2,
  textShadowOpacity: 0.85,
  shadowOnText: true,
  shadowOnNames: true,
  shadowOnBadges: false,
  shadowDarkText: false,
  shadowDarkNames: false,
  shadowDarkBadges: false,
  textAlign: 'left',
  fontWeight: 400,
  textItalic: false,
  textStrikethrough: false,
  background: 'transparent',
  backgroundColor: '#0e0e10',
  backgroundOpacity: 0.8,
  eventStyle: 'plain',
  eventFill: false,
  eventAnimation: 'none',
  eventAnimateRepeat: false,
  cheerDisplay: 'message',
  seventvEmoteChanges: true,
  hideBots: false,
  hiddenEvents: [],
  hiddenProviderEvents: [],
  superchatCurrency: '',
  blockedUsers: {},
  hideCommands: false,
  commandFilters: [{ value: '!', mode: 'prefix' }],
  showPaints: true,
  showThirdPartyBadges: true,
  showAtmospheres: true,
  hiddenBadgeProviders: [],
  hiddenTwitchBadgeCategories: [],
  ffzCustomRoleBadges: false,
  emojiStyle: 'apple',
  bubble: false,
  bubbleShape: 'rounded',
  bubbleRadius: 10,
  bubbleColor: '#0e0e10',
  bubbleOpacity: 0.55,
  firstTimeColor: '',
  eventOutlineColor: '',
  hidePhrases: [],
  maxMessageAgeSec: 0,
  maxMessageLines: 0,
  showAvatars: true,
  showAtSign: true,
  readableNameColors: true,
  showReplies: true,
  replyStyle: 'full',
  linkStyle: 'accent',
  linkColor: '',
  linkUnderline: true,
  showPersonalEmotes: true,
  eventTemplates: {},
  firstTimeStyle: 'off',
  firstTimeFill: false,
  firstTimeAnimation: 'none',
  firstTimeAnimateRepeat: false,
  direction: 'newBottom',
  entrance: 'fade',
  restoreOnReload: false,
  loadRecentChat: false,
  modCommands: true,
  giantEmotes: true,
  giantEmoteAlign: 'center',
  showGifs: true,
  gifScale: 1,
  showUploadedImages: false,
  uploadedImageScale: 1,
  uploadedImagesFrom: 'trusted',
};

// Clamp ranges so a builder (or a hand-edited saved config) can't produce a
// broken overlay. Mirrored by the renderer.
export const OVERLAY_LIMITS = {
  // Floors are deliberately low. The renderer measures real row heights and mounts
  // only what fits, so a short canvas just shows fewer messages — nothing breaks.
  // A low banner strip across a gameplay scene is a real use case.
  width: { min: 160, max: 900 },
  height: { min: 80, max: 1600 },
  fontSize: { min: 10, max: 48 },
  lineHeight: { min: 1, max: 2.2 },
  messageGap: { min: 0, max: 28 },
  emoteScale: { min: 0.5, max: 3 },
  gifScale: { min: 0.5, max: 2 },
  uploadedImageScale: { min: 0.5, max: 3 },
  badgeScale: { min: 0.5, max: 2.5 },
  fontWeight: { min: 300, max: 700 },
  textShadowSize: { min: 0, max: 12 },
  textShadowOpacity: { min: 0, max: 1 },
  backgroundOpacity: { min: 0, max: 1 },
  bubbleOpacity: { min: 0.05, max: 1 },
  bubbleRadius: { min: 0, max: 24 },
  maxMessageAgeSec: { min: 0, max: 600 },
  maxMessageLines: { min: 0, max: 6 },
} as const;

/** Digits after the point in a number as written (0.05 → 2, 10 → 0). */
export const decimalsOf = (n: number): number => {
  const s = String(+n.toFixed(6));
  const dot = s.indexOf('.');
  return dot < 0 ? 0 : s.length - dot - 1;
};

/**
 * A number typed into a builder slider's value, clamped to the range and
 * snapped to the step the way the slider itself would land. `scale` turns what
 * the field shows back into the stored value (a 0 to 1 strength shown as 85%
 * types as 85). Null for text that is not a number, so the field reverts.
 */
export function parseSliderInput(text: string, opts: { min: number; max: number; step: number; scale?: number }): number | null {
  const n = parseFloat(text.trim().replace(',', '.'));
  if (!Number.isFinite(n)) return null;
  const { min, max, step } = opts;
  const scale = opts.scale ?? 1;
  const clamp = (v: number) => Math.min(max, Math.max(min, v));
  const snapped = min + Math.round((clamp(n / scale) - min) / step) * step;
  return clamp(+snapped.toFixed(decimalsOf(step)));
}

/** Longest hidden phrase or command filter the builder accepts as a new entry. */
export const OVERLAY_TEXT_ENTRY_MAX = 200;

/** Longest hidden phrase or command filter an overlay file carries. Entries
 *  saved before new ones were capped at OVERLAY_TEXT_ENTRY_MAX can be longer;
 *  they travel whole up to this length, and a longer one is left out and
 *  reported. The desktop file reader (overlay_file.rs) uses the same number. */
export const OVERLAY_FILE_TEXT_ENTRY_MAX = 1000;

/** The most characters a published overlay's style may serialize to. The
 *  overlay server refuses a bigger one (style_too_large), measuring it as the
 *  length of JSON.stringify of the style it receives. */
export const OVERLAY_STYLE_MAX_CHARS = 16_000;

/** Whether a style, exactly as it is sent to the server, is over the server's cap. */
export const overlayStyleTooLarge = (style: unknown): boolean =>
  JSON.stringify(style ?? {}).length > OVERLAY_STYLE_MAX_CHARS;

// Coerce commandFilters into valid { value, mode } entries. Repairs legacy shapes
// (a plain string from an earlier version → inferred mode) and drops empties, so a
// stale saved config can never render blank/garbage chips or filter on nothing.
export function sanitizeCommandFilters(raw: unknown): { value: string; mode: 'prefix' | 'exact' }[] {
  if (!Array.isArray(raw)) return [];
  const out: { value: string; mode: 'prefix' | 'exact' }[] = [];
  const seen = new Set<string>();
  for (const item of raw) {
    let value = '';
    let mode: 'prefix' | 'exact' = 'prefix';
    if (typeof item === 'string') {
      value = item.trim();
      mode = /[a-z0-9]/i.test(value) ? 'exact' : 'prefix';
    } else if (item && typeof item === 'object') {
      const rec = item as { value?: unknown; mode?: unknown };
      value = String(rec.value ?? '').trim();
      mode = rec.mode === 'exact' ? 'exact' : 'prefix';
    }
    if (!value) continue;
    const key = `${mode}:${value.toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ value, mode });
  }
  return out;
}

// Shadow keys an earlier build saved: the separate badge and name shadows (the
// one shadow and its shadowOn* switches replaced them) and the shared Soft /
// Thick choice (shadowDark* replaced it). A save drops them.
const RETIRED_SHADOW_KEYS = [
  'badgeShadow', 'badgeShadowColor', 'badgeShadowSize', 'badgeShadowOpacity',
  'nameShadow', 'nameShadowColor', 'nameShadowSize', 'nameShadowOpacity', 'nameShadowStyle',
  'textShadowStyle',
] as const;

// What the shadow sits behind. A config saved before the switches existed drew
// the text shadow behind text and names, plus badges or names on their own
// switch: fold those in, and where a part had its own shadow while the text
// shadow was off, turn the shadow on for just that part, so the overlay draws
// the shadows it drew.
const shadowParts = (s: OverlayStyle): Pick<OverlayStyle, 'textShadow' | 'shadowOnText' | 'shadowOnNames' | 'shadowOnBadges'> => {
  if (typeof s.shadowOnText === 'boolean') {
    return { textShadow: s.textShadow, shadowOnText: s.shadowOnText, shadowOnNames: s.shadowOnNames !== false, shadowOnBadges: s.shadowOnBadges === true };
  }
  const old = s as { badgeShadow?: unknown; nameShadow?: unknown };
  const badges = old.badgeShadow === true;
  const names = old.nameShadow === true;
  const partOnly = s.textShadow === false && (badges || names);
  return { textShadow: partOnly ? true : s.textShadow, shadowOnText: !partOnly, shadowOnNames: partOnly ? names : true, shadowOnBadges: badges };
};

export const clampOverlayStyle = (s: OverlayStyle): OverlayStyle => {
  const clamp = (v: number, lo: number, hi: number) =>
    Math.min(hi, Math.max(lo, v));
  // Phrases: trimmed, non-empty, deduped case-insensitively, bounded so a
  // hand-edited config can't ship an absurd list.
  const phrases: string[] = [];
  const seenPhrases = new Set<string>();
  for (const p of Array.isArray(s.hidePhrases) ? s.hidePhrases : []) {
    const v = String(p ?? '').trim();
    const key = v.toLowerCase();
    if (!v || seenPhrases.has(key)) continue;
    seenPhrases.add(key);
    phrases.push(v);
    if (phrases.length >= 100) break;
  }
  // Event filtering is per-source (hiddenProviderEvents, keyed `provider:category`).
  // Fold the legacy global hiddenEvents in: each global hide expands to that
  // category on every platform that can emit it, then the global list is dropped.
  // Idempotent once migrated, so it is safe on every render (preview + hosted).
  const providerEvents = new Set(
    (Array.isArray(s.hiddenProviderEvents) ? s.hiddenProviderEvents : [])
      .filter((k) => typeof k === 'string' && k.includes(':')),
  );
  for (const cat of Array.isArray(s.hiddenEvents) ? s.hiddenEvents : []) {
    for (const [provider, cats] of Object.entries(PROVIDER_EVENT_CATEGORIES)) {
      if ((cats ?? []).includes(cat as EventCategory)) providerEvents.add(`${provider}:${cat}`);
    }
  }
  const kept: OverlayStyle & Record<string, unknown> = { ...s };
  for (const key of RETIRED_SHADOW_KEYS) delete kept[key];
  return {
    ...kept,
    commandFilters: sanitizeCommandFilters(s.commandFilters),
    hidePhrases: phrases,
    hiddenEvents: [],
    hiddenProviderEvents: Array.from(providerEvents),
    // Absent on configs saved before badge categories existed → every badge shows.
    hiddenTwitchBadgeCategories: Array.from(new Set(
      (Array.isArray(s.hiddenTwitchBadgeCategories) ? s.hiddenTwitchBadgeCategories : [])
        .filter((id) => TWITCH_BADGE_CATEGORIES.some((c) => c.id === id)),
    )),
    bubbleOpacity: clamp(s.bubbleOpacity ?? 0.55, OVERLAY_LIMITS.bubbleOpacity.min, OVERLAY_LIMITS.bubbleOpacity.max),
    bubbleRadius: Math.round(clamp(s.bubbleRadius ?? 10, OVERLAY_LIMITS.bubbleRadius.min, OVERLAY_LIMITS.bubbleRadius.max)),
    maxMessageAgeSec: Math.round(clamp(s.maxMessageAgeSec ?? 0, OVERLAY_LIMITS.maxMessageAgeSec.min, OVERLAY_LIMITS.maxMessageAgeSec.max)),
    maxMessageLines: Math.round(clamp(s.maxMessageLines ?? 0, OVERLAY_LIMITS.maxMessageLines.min, OVERLAY_LIMITS.maxMessageLines.max)),
    width: Math.round(clamp(s.width, OVERLAY_LIMITS.width.min, OVERLAY_LIMITS.width.max)),
    height: Math.round(clamp(s.height, OVERLAY_LIMITS.height.min, OVERLAY_LIMITS.height.max)),
    fontSize: clamp(s.fontSize, OVERLAY_LIMITS.fontSize.min, OVERLAY_LIMITS.fontSize.max),
    lineHeight: clamp(s.lineHeight, OVERLAY_LIMITS.lineHeight.min, OVERLAY_LIMITS.lineHeight.max),
    messageGap: clamp(s.messageGap, OVERLAY_LIMITS.messageGap.min, OVERLAY_LIMITS.messageGap.max),
    emoteScale: clamp(s.emoteScale, OVERLAY_LIMITS.emoteScale.min, OVERLAY_LIMITS.emoteScale.max),
    badgeScale: clamp(s.badgeScale, OVERLAY_LIMITS.badgeScale.min, OVERLAY_LIMITS.badgeScale.max),
    backgroundOpacity: clamp(s.backgroundOpacity, OVERLAY_LIMITS.backgroundOpacity.min, OVERLAY_LIMITS.backgroundOpacity.max),
    // Absent on configs saved before the field existed → on (the Twitch-faithful default).
    giantEmotes: s.giantEmotes !== false,
    // Absent on configs saved before these existed → today's look, so an overlay
    // published before the update renders exactly as it did.
    giantEmoteAlign:
      s.giantEmoteAlign === 'left' || s.giantEmoteAlign === 'right' || s.giantEmoteAlign === 'inline'
        ? s.giantEmoteAlign
        : 'center',
    // Absent on configs saved before chat GIFs existed → on, like Twitch.
    showGifs: s.showGifs !== false,
    // Absent on configs saved before the switch existed → Twitch's own art, the
    // look those overlays always had.
    ffzCustomRoleBadges: s.ffzCustomRoleBadges === true,
    // Absent on configs saved before 7TV emote rows existed → shown.
    seventvEmoteChanges: s.seventvEmoteChanges !== false,
    // Absent on configs saved before the GIF size existed → 1, today's size.
    gifScale: clamp(
      Number.isFinite(s.gifScale) ? s.gifScale : 1,
      OVERLAY_LIMITS.gifScale.min,
      OVERLAY_LIMITS.gifScale.max,
    ),
    // Absent on configs saved before uploaded pictures existed: off, so every
    // link stays text as it always did.
    showUploadedImages: s.showUploadedImages === true,
    uploadedImageScale: clamp(
      Number.isFinite(s.uploadedImageScale) ? s.uploadedImageScale : 1,
      OVERLAY_LIMITS.uploadedImageScale.min,
      OVERLAY_LIMITS.uploadedImageScale.max,
    ),
    uploadedImagesFrom: s.uploadedImagesFrom === 'everyone' ? 'everyone' : 'trusted',
    textAlign: s.textAlign === 'center' || s.textAlign === 'right' ? s.textAlign : 'left',
    cheerDisplay: s.cheerDisplay === 'event' ? 'event' : 'message',
    fontWeight:
      Math.round(clamp(s.fontWeight ?? 400, OVERLAY_LIMITS.fontWeight.min, OVERLAY_LIMITS.fontWeight.max) / 100) * 100,
    textItalic: s.textItalic === true,
    textStrikethrough: s.textStrikethrough === true,
    textShadowColor: (s.textShadowColor || '').trim() || '#000000',
    textShadowSize: clamp(s.textShadowSize ?? 2, OVERLAY_LIMITS.textShadowSize.min, OVERLAY_LIMITS.textShadowSize.max),
    textShadowOpacity: clamp(s.textShadowOpacity ?? 0.85, OVERLAY_LIMITS.textShadowOpacity.min, OVERLAY_LIMITS.textShadowOpacity.max),
    // Dark replaced a shared Soft / Thick choice, which only changed text and names.
    shadowDarkText: typeof s.shadowDarkText === 'boolean' ? s.shadowDarkText : (s as { textShadowStyle?: unknown }).textShadowStyle === 'thick',
    shadowDarkNames: typeof s.shadowDarkNames === 'boolean' ? s.shadowDarkNames : (s as { textShadowStyle?: unknown }).textShadowStyle === 'thick',
    shadowDarkBadges: s.shadowDarkBadges === true,
    ...shadowParts(s),
    // Reply rendering moved from a boolean to a three-way. A config saved before
    // replyStyle existed only knows showReplies, so read that: false → 'off',
    // anything else → the context line it used to draw. Idempotent once migrated.
    replyStyle:
      s.replyStyle === 'mention' || s.replyStyle === 'off' || s.replyStyle === 'full'
        ? s.replyStyle
        : s.showReplies === false
          ? 'off'
          : 'full',
    // Absent on configs saved before these existed → today's look, so an overlay
    // published before the update renders exactly as it did.
    linkStyle: s.linkStyle === 'plain' ? 'plain' : 'accent',
    linkColor: (s.linkColor || '').trim(),
    linkUnderline: s.linkUnderline !== false,
    showPersonalEmotes: s.showPersonalEmotes !== false,
    eventTemplates: sanitizeEventTemplates(s.eventTemplates),
  };
};

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------
// Installed fonts, matched by every name a streamer might type.
//
// CSS `font-family` only ever matches an installed font by its FAMILY name
// ("Montserrat"). The names Windows shows in its font viewer and in file names
// are often FACE names ("Montserrat Bold", "Uni Sans Heavy Caps") or PostScript
// names ("Montserrat-Bold"), and Chromium (OBS's browser source, WebView2,
// every browser) never matches those as a family, so the overlay fell back to
// sans-serif. `local()` inside an @font-face does match full face names and
// PostScript names. So a custom font gets an alias family whose sources are
// local() lookups under each of those spellings, placed right after the typed
// name in the stack: a real family name (installed or a webfont) still wins,
// and a face or PostScript name resolves too.

/** Generic CSS family keywords: never a webfont load, never an alias. */
const GENERIC_FONT_FAMILIES = new Set([
  'system-ui', 'ui-sans-serif', 'ui-monospace', 'ui-serif', 'sans-serif', 'serif',
  'monospace', 'cursive', 'fantasy', '-apple-system', 'blinkmacsystemfont',
  'inherit', 'initial', 'unset',
]);

/** A font-family list split into its families at the commas outside quotes,
 *  each trimmed, or null when a quote never closes. */
export function splitFontFamilies(ff: string): string[] | null {
  const parts: string[] = [];
  let cur = '';
  let quote: string | null = null;
  for (let i = 0; i < ff.length; i++) {
    const c = ff[i];
    if (quote) {
      cur += c;
      if (c === '\\' && i + 1 < ff.length) cur += ff[++i];
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") {
      quote = c;
      cur += c;
    } else if (c === ',') {
      parts.push(cur.trim());
      cur = '';
    } else {
      cur += c;
    }
  }
  if (quote) return null;
  parts.push(cur.trim());
  return parts.filter(Boolean);
}

/** First family in a font-family string, unquoted. */
export const primaryFontFamily = (ff: string): string => {
  const first = (splitFontFamilies(ff || '') ?? [(ff || '').split(',')[0]])[0] ?? '';
  const quoted = /^(["'])([\s\S]*)\1$/.exec(first.trim());
  return quoted ? quoted[2] : first.trim().replace(/^["']|["']$/g, '');
};

/** A named family rather than a generic keyword. */
export const isCustomFontFamily = (fam: string): boolean =>
  !!fam && !GENERIC_FONT_FAMILIES.has(fam.toLowerCase());

/** FNV-1a over the name's UTF-16 code units, in base 36. */
const shortHash = (s: string): string => {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(36);
};

/** The alias family that finds an installed font by its face or PostScript name.
 *  The readable part keeps only ASCII letters and digits, so the hash of the
 *  full name keeps two names apart that share it (every Japanese name, or
 *  "游ゴシック Medium" and "游明朝 Medium"). */
export const localFontAlias = (family: string): string => {
  const slug = family.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 40).replace(/^-+|-+$/g, '');
  return `sn-local-${slug ? `${slug}-` : ''}${shortHash(family)}`;
};

const cssString = (s: string): string =>
  `'${s.replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/[\n\r\f]/g, ' ')}'`;

/** The @font-face rule behind localFontAlias: local() under the typed name, its
 *  Regular face, and the PostScript spellings (spaces dropped, "-Regular"). The
 *  face claims every weight, so a name typed as a Bold face draws bold names
 *  in that face as it is instead of thickening it a second time. */
export function localFontFaceCss(family: string): string {
  const compact = family.replace(/\s+/g, '');
  const names = Array.from(new Set([family, `${family} Regular`, compact, `${compact}-Regular`]));
  return `@font-face{font-family:${cssString(localFontAlias(family))};src:${names.map((n) => `local(${cssString(n)})`).join(',')};font-weight:100 900;}`;
}

/** The alias families to keep declared after `family` was used: most recently
 *  used last, at most `max`, so the oldest gives way first. */
export function touchLocalFontAliases(current: readonly string[], family: string, max: number): string[] {
  const next = current.filter((f) => f !== family);
  next.push(family);
  return next.length > max ? next.slice(next.length - max) : next;
}

/** The font-family stack the overlay renders with: the chosen stack with the
 *  installed-font alias right after its first family. Generic stacks
 *  ('system-ui, sans-serif') and lists with an unclosed quote come back
 *  unchanged. */
export function overlayFontStack(fontFamily: string): string {
  const parts = splitFontFamilies(fontFamily || '');
  if (!parts || parts.length === 0) return fontFamily;
  const fam = primaryFontFamily(fontFamily);
  if (!isCustomFontFamily(fam)) return fontFamily;
  return [parts[0], `'${localFontAlias(fam)}'`, ...parts.slice(1)].join(', ');
}

// ---------------------------------------------------------------------------
// 7TV emote set changes
// ---------------------------------------------------------------------------

/** One change to a channel's 7TV emote set, as an overlay row shows it. */
export interface SeventvEmoteChange {
  action: 'added' | 'removed' | 'renamed';
  /** Who made the change on 7TV. */
  actor: string;
  /** The emote's name in the set (the new name for a rename). */
  name: string;
  /** The name before a rename. */
  oldName?: string;
  /** The emote's image, when known. */
  emoteUrl?: string;
}

/** A 7TV emote's image by id, at the size the overlay draws emotes. */
export const seventvEmoteImageUrl = (id: string): string => `https://cdn.7tv.app/emote/${id}/3x.webp`;

/**
 * The changes in one 7TV EventAPI `emote_set.update` body: `pushed` rows were
 * added, `pulled` rows removed, and an `updated` row whose name changed was
 * renamed (a flag-only update is not a change a viewer sees).
 */
export function seventvChangesFromDispatch(body: unknown): SeventvEmoteChange[] {
  const b = body as {
    actor?: { display_name?: unknown; username?: unknown } | null;
    pushed?: unknown;
    pulled?: unknown;
    updated?: unknown;
  } | null;
  if (!b || typeof b !== 'object') return [];
  const actorRaw = b.actor?.display_name || b.actor?.username;
  const actor = typeof actorRaw === 'string' && actorRaw.trim() ? actorRaw.trim() : 'Someone';
  type Row = { key?: unknown; value?: unknown; old_value?: unknown };
  const rows = (list: unknown): Row[] =>
    Array.isArray(list) ? (list as Row[]).filter((c) => c && c.key === 'emotes') : [];
  const emote = (v: unknown): { id?: string; name?: string } => {
    const e = v as { id?: unknown; name?: unknown; data?: { id?: unknown } } | null;
    const id = typeof e?.id === 'string' ? e.id : typeof e?.data?.id === 'string' ? e.data.id : undefined;
    return { id, name: typeof e?.name === 'string' ? e.name : undefined };
  };
  const out: SeventvEmoteChange[] = [];
  for (const r of rows(b.pushed)) {
    const e = emote(r.value);
    if (e.name) out.push({ action: 'added', actor, name: e.name, ...(e.id ? { emoteUrl: seventvEmoteImageUrl(e.id) } : {}) });
  }
  for (const r of rows(b.pulled)) {
    const e = emote(r.old_value);
    if (e.name) out.push({ action: 'removed', actor, name: e.name, ...(e.id ? { emoteUrl: seventvEmoteImageUrl(e.id) } : {}) });
  }
  for (const r of rows(b.updated)) {
    const before = emote(r.old_value);
    const after = emote(r.value);
    if (!after.name || !before.name || after.name === before.name) continue;
    const id = after.id ?? before.id;
    out.push({ action: 'renamed', actor, name: after.name, oldName: before.name, ...(id ? { emoteUrl: seventvEmoteImageUrl(id) } : {}) });
  }
  return out;
}

// ---------------------------------------------------------------------------
// FrankerFaceZ room moderator and VIP art
// ---------------------------------------------------------------------------

/** FFZ's moderator art is a white glyph its own client draws on the moderator
 *  badge's green square. */
export const FFZ_MOD_BACKDROP = '#34AE0A';

const FFZ_ROOM_BADGE_CDN = 'https://cdn.frankerfacez.com/room-badge/';

/**
 * The largest image of a channel's FFZ moderator and VIP art, from its room
 * payload (`/v1/room/id/{id}`: `room.mod_urls`, `room.vip_badge`, each
 * `{"1","2","4"}`). Rooms that list only "1" still serve every scale on FFZ's
 * room-badge CDN, so the 4x address is built from the listed one. Null for a
 * role the room does not replace.
 */
export function ffzRoomRoleArt(payload: unknown): { moderator: string | null; vip: string | null } {
  const room = (payload as { room?: Record<string, unknown> } | null)?.room;
  const pick = (urls: unknown): string | null => {
    if (!urls || typeof urls !== 'object') return null;
    const listed = (scale: string): string | null => {
      const u = (urls as Record<string, unknown>)[scale];
      return typeof u === 'string' && u.startsWith('https://') ? u : null;
    };
    const direct = listed('4');
    if (direct) return direct;
    const any = listed('2') ?? listed('1');
    if (!any) return null;
    return any.startsWith(FFZ_ROOM_BADGE_CDN) ? `${any.slice(0, any.lastIndexOf('/'))}/4` : any;
  };
  return { moderator: pick(room?.mod_urls), vip: pick(room?.vip_badge) };
}

/** Width and height from a PNG's header, or null for bytes that are not a PNG. */
export function pngDimensions(bytes: Uint8Array): { w: number; h: number } | null {
  const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (bytes.length < 24 || sig.some((b, i) => bytes[i] !== b)) return null;
  if (String.fromCharCode(bytes[12], bytes[13], bytes[14], bytes[15]) !== 'IHDR') return null;
  const read = (o: number) => ((bytes[o] << 24) | (bytes[o + 1] << 16) | (bytes[o + 2] << 8) | bytes[o + 3]) >>> 0;
  const w = read(16);
  const h = read(20);
  return w > 0 && h > 0 ? { w, h } : null;
}

/** A glyph PNG on a rounded square of `color` (radius side / 9, FFZ's own
 *  shape), as an SVG data URI: the same finished badge the app paints. Null
 *  for bytes that are not a PNG or a colour that is not `#RRGGBB`. */
export function badgeWithBackdrop(png: Uint8Array, color: string): string | null {
  if (!/^#[0-9a-f]{6}$/i.test(color)) return null;
  const size = pngDimensions(png);
  if (!size) return null;
  let bin = '';
  for (let i = 0; i < png.length; i++) bin += String.fromCharCode(png[i]);
  const glyph = btoa(bin);
  const radius = Math.min(size.w, size.h) / 9;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size.w}" height="${size.h}" viewBox="0 0 ${size.w} ${size.h}">` +
    `<rect width="${size.w}" height="${size.h}" rx="${radius}" fill="${color}"/>` +
    `<image width="${size.w}" height="${size.h}" href="data:image/png;base64,${glyph}"/></svg>`;
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}
