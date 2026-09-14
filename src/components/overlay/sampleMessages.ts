// Realistic sample feed for the overlay builder preview. These mirror the exact
// BackendChatMessage shape the live overlay will receive over the WebSocket, so
// the preview renders through the same path as real data. Paints are pure-CSS
// 7TV-style gradients (no external asset, so they always render); emotes/badges
// use stable Twitch CDN URLs so the demo never shows broken images.

import type { BackendChatMessage, MessageSegment } from '../../services/twitchChat';
import type { ProviderId } from '../../types/providers';

// Structural subset of the 7TV PaintV4 shape (that type is module-private in
// seventvService). Assignable to computePaintStyle's parameter, so the preview
// runs the real paint→CSS helper rather than a stand-in.
export interface OverlayPaint {
  id: string;
  name: string;
  data: {
    layers: Array<{
      id: string;
      opacity: number;
      ty: {
        __typename: string;
        angle?: number;
        repeating?: boolean;
        stops?: Array<{ at: number; color: { hex: string; r: number; g: number; b: number; a: number } }>;
        color?: { hex: string; r: number; g: number; b: number; a: number };
      };
    }>;
    shadows: Array<{ offsetX: number; offsetY: number; blur: number; color: { hex: string; r: number; g: number; b: number; a: number } }>;
  };
  selected?: boolean;
}

/** Portable subset of the app's Atmosphere used to render the member's chat wash
 *  on the overlay — pure CSS descriptors, no app/Tauri deps, so the same renderer
 *  runs in the builder and on the hosted page. Resolved per host (app store
 *  in-app, Supabase/identity on the hosted page). The rare CS2 'cologne-chrome'
 *  atmosphere is intentionally omitted from the overlay. */
export interface OverlayAtmosphere {
  baseColor: string;
  baseLayers?: string;
  image?: string;
  layers?: string;
  layers2?: string;
  /** 1px gradient edge down the left of the row ('none' = no edge). */
  chatEdge: string;
  /** Frost the text block for readability over a busy (image) wash. */
  chatFrost?: boolean;
  /** Defocus (px) for an image wash, so it reads as dispersed light. */
  chatBlur?: number;
  /** 1px gradient rim around the row. */
  chatRim?: string;
}

/** A preview message: the real backend shape plus the cosmetics the live overlay
 *  attaches after resolution (paint, 7TV badge, third-party badges, StreamNook
 *  membership). The hosted overlay will receive these pre-resolved from the
 *  backend; in-app they come from the real chat pipeline. */
export type OverlayMessage = BackendChatMessage & {
  paint?: OverlayPaint;
  /** Resolved 7TV badge image URL (+ title). */
  seventvBadgeUrl?: string;
  seventvBadgeTitle?: string;
  /** Resolved third-party (FFZ / Chatterino / Homies / …) badge images. */
  extraBadges?: { url: string; title?: string; source?: string }[];
  /** StreamNook member number → renders the StreamNook identity badge. Null or
   *  absent means not a StreamNook member. */
  streamNookUserNumber?: number | null;
  /** Resolved StreamNook cosmetic badge image URL (the member's equipped badge);
   *  absent → the default StreamNook logo. */
  streamNookBadgeUrl?: string | null;
  /** Resolved StreamNook Atmosphere → renders the member's animated wash behind
   *  the row. Absent/null means none. */
  atmosphere?: OverlayAtmosphere | null;
  /** Set by the feed when moderation deletes this message: the renderer fades
   *  the row out, then the feed removes it for real a beat later. */
  retracted?: boolean;
};

const hex = (h: string) => {
  const n = parseInt(h.slice(1), 16);
  return { hex: h, r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255, a: 1 };
};

// Build a linear-gradient paint (the most common 7TV paint style).
const linearPaint = (
  id: string,
  name: string,
  angle: number,
  stops: Array<[number, string]>,
  shadow?: [string, number, number, number],
): OverlayPaint => ({
  id,
  name,
  data: {
    layers: [
      {
        id: `${id}-l0`,
        opacity: 1,
        ty: {
          __typename: 'PaintLayerTypeLinearGradient',
          angle,
          stops: stops.map(([at, color]) => ({ at, color: hex(color) })),
        },
      },
    ],
    shadows: shadow
      ? [{ color: hex(shadow[0]), offsetX: shadow[1], offsetY: shadow[2], blur: shadow[3] }]
      : [],
  },
  selected: true,
});

const TWITCH_BADGE = (uuid: string) => `https://static-cdn.jtvnw.net/badges/v1/${uuid}/3`;
const TWITCH_EMOTE = (id: string) => `https://static-cdn.jtvnw.net/emoticons/v2/${id}/default/dark/3.0`;

const t = (content: string): MessageSegment => ({ type: 'text', content });
const e = (content: string, id: string): MessageSegment => ({
  type: 'emote',
  content,
  emote_id: id,
  emote_url: TWITCH_EMOTE(id),
});

// A cheermote (bits) segment, as the live tokenizers emit one. `prefix` is
// lowercased there, and a channel's own prefix can contain digits.
const cm = (content: string, prefix: string, bits: number, tier: string, color: string): MessageSegment => ({
  type: 'cheermote',
  content,
  prefix,
  bits,
  tier,
  color,
  cheermote_url: `https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/${tier}/2.gif`,
});

// Shared empty scaffolding so each sample stays terse.
const base = (
  id: string,
  provider: ProviderId,
  username: string,
  display_name: string,
  color: string,
  segments: MessageSegment[],
  extra: Partial<OverlayMessage> = {},
): OverlayMessage => ({
  id,
  username,
  display_name,
  color,
  user_id: `uid-${id}`,
  timestamp: new Date(0).toISOString(),
  content: segments.map((s) => ('content' in s ? s.content : '')).join(' '),
  provider,
  channel: provider === 'twitch' ? 'sodapoppin' : `${provider}:sample`,
  badges: [],
  emotes: [],
  layout: { height: 0, width: 0 },
  tags: {},
  segments,
  metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, formatted_timestamp: '9:41 PM' },
  ...extra,
});

// A rainbow-ish 7TV-style paint and a fiery one, plus a StreamNook-native look.
const PAINT_NEBULA = linearPaint('paint-nebula', 'Nebula', 120, [
  [0, '#8a5cff'], [0.5, '#ff5ca8'], [1, '#5cc8ff'],
], ['#000000', 0, 1, 2]);
const PAINT_EMBER = linearPaint('paint-ember', 'Ember', 90, [
  [0, '#ffd16a'], [0.6, '#ff7a3c'], [1, '#ff2e63'],
]);
const PAINT_SN = linearPaint('paint-sn', 'Winters Glass', 135, [
  [0, '#bfe3ff'], [0.5, '#7fb2ff'], [1, '#a98bff'],
], ['#0a1830', 0, 1, 3]);

// The REAL "Aurora" atmosphere (verbatim snapshot of the Supabase `atmospheres`
// row, camelCased), so the preview shows exactly what the live overlay + the
// in-app chat render — not a stand-in. Keep in sync if the Aurora row changes.
const SAMPLE_ATMOSPHERE: OverlayAtmosphere = {
  baseColor: '#04070c',
  baseLayers: 'radial-gradient(ellipse 120% 60% at 50% 0%, rgba(45,212,191,0.10), transparent 65%)',
  layers: 'repeating-linear-gradient(95deg, rgba(16,185,129,0) 0px, rgba(45,212,191,0.18) 120px, rgba(34,211,238,0.10) 200px, rgba(16,185,129,0) 320px)',
  layers2: 'repeating-linear-gradient(88deg, rgba(34,211,238,0) 0px, rgba(34,211,238,0.12) 90px, rgba(139,92,246,0.10) 160px, rgba(34,211,238,0) 240px)',
  chatEdge: 'linear-gradient(to bottom, transparent, rgba(45,212,191,0.85) 22%, rgba(34,211,238,0.75) 52%, rgba(139,92,246,0.6) 80%, transparent)',
  chatFrost: false,
};

export const SAMPLE_MESSAGES: OverlayMessage[] = [
  base('m1', 'twitch', 'sodapoppin', 'sodapoppin', '#e0457b',
    [t('chat be normal for once challenge'), e('Kappa', '25'), t('(impossible)')],
    {
      paint: PAINT_NEBULA,
      badges: [
        { name: 'broadcaster', version: '1', image_url_4x: TWITCH_BADGE('5527c58c-fb7d-422d-b71b-f309dcb85cc1'), title: 'Broadcaster' },
      ],
    },
  ),
  base('m2', 'twitch', 'nmplol', 'Nmplol', '#1e90ff',
    [t('mods are asleep, post furbies'), e('PogChamp', '305954156')],
    {
      badges: [
        { name: 'moderator', version: '1', image_url_4x: TWITCH_BADGE('3267646d-33f0-4b17-b3df-f923a41db1d0'), title: 'Moderator' },
        { name: 'premium', version: '1', image_url_4x: TWITCH_BADGE('bbbe0db0-a598-423e-86d0-f9fb98ca1933'), title: 'Prime Gaming' },
      ],
    },
  ),
  base('m3', 'kick', 'trainwreckstv', 'Trainwreckstv', '#53fc18',
    [t('kick chat single-handedly lowering the average IQ'), e('LUL', '425618')],
  ),
  // YouTube names arrive as @handles, so this one previews the "@ before
  // usernames" toggle (and the avatar previews "Profile pictures").
  base('m4', 'youtube', '@ludwig', '@Ludwig', '#ff5c5c',
    [t('youtube gang showing up 4 hours late as usual')],
    { tags: { avatar: 'https://static-cdn.jtvnw.net/jtv_user_pictures/xarth/404_user_70x70.png' } },
  ),
  base('m5', 'tiktok', 'poki', 'Pokimane', '#00f2ea',
    [t('sending 600 hearts instead of reading the question'), t('💗💗')],
  ),
  base('m6', 'twitch', 'winters', 'Winters', '#a98bff',
    [t('yes my name is glowing, no you cannot have it. the whole chat sees my paint even though none of you run StreamNook')],
    {
      paint: PAINT_SN,
      streamNookUserNumber: 42,
      streamNookBadgeUrl: 'https://streamnook.app/cosmetics/streamnook-badge-gold.png',
      atmosphere: SAMPLE_ATMOSPHERE,
      badges: [
        { name: 'vip', version: '1', image_url_4x: TWITCH_BADGE('b817aba4-fad8-49e2-b88a-7cc744dfa6ec'), title: 'VIP' },
      ],
    },
  ),
  base('m7', 'twitch', 'esfandtv', 'Esfand', '#ffab40',
    [t('this is already going great')],
    {
      paint: PAINT_EMBER,
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:42 PM',
        reply_info: {
          parent_msg_id: 'm1', parent_display_name: 'sodapoppin', parent_user_id: 'uid-m1',
          parent_user_login: 'sodapoppin', parent_msg_body: 'chat be normal for once challenge (impossible)',
        },
      },
    },
  ),
  base('m8', 'twitch', 'emoteonly', 'EmoteOnly', '#57c2a3',
    [e('Kappa', '25'), e('PogChamp', '305954156'), e('LUL', '425618')],
  ),
  // A Twitch chat GIF (Tier 2/3 subscribers): the bracketed description Twitch
  // puts in the text plus the GIPHY asset from the `gifs` tag, taken verbatim
  // from Twitch's documented example so the preview shows the real shape.
  base('m8b', 'twitch', 'gifdropper', 'GifDropper', '#f2a65a',
    [
      t('this stream is'),
      {
        type: 'gif',
        content: '[Y A Y Yes GIF by Djemilah Birnie]',
        gif_id: 'joSNxeswxuc74Juo8X',
        gif_url: 'https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?cid=095d7a5dzizsiwgabonagkmigggv8v1spfai91ac3x0dsiy0&ep=v1_gifs_trending&rid=giphy.gif&ct=g',
      },
    ],
  ),
  base('m9', 'twitch', 'gifterpro', 'GifterPro', '#7bd88f',
    [t('twelve months and he still hasnt beaten the tutorial'), e('PogChamp', '305954156')],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:43 PM',
        msg_type: 'resub',
        system_message: "GifterPro subscribed at Tier 1. They've subscribed for 12 months!",
      },
    },
  ),
  // Cross-platform events — every provider speaks the same system-msg vocabulary,
  // so they render through the same event row (YouTube superchat, TikTok gift).
  base('m10', 'youtube', 'ytfan', 'YT Fan', '#ff5c5c', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:44 PM',
        msg_type: 'superchat',
        system_message: 'YT Fan sent a $10.00 Super Chat: quit your job and stream full time',
      },
    },
  ),
  base('m11', 'tiktok', 'ttuser', 'TikTok User', '#00f2ea', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:45 PM',
        msg_type: 'tiktok_gift',
        system_message: 'TikTok User sent Rose x5',
      },
    },
  ),
  // YouTube events (membership, gift membership, super sticker) + Kick events (sub,
  // gift, follow) — same event rows as Twitch/TikTok, tagged to their platform so a
  // merged overlay shows each provider's events consistently.
  base('m21', 'youtube', 'ytmember', 'YT Member', '#ff5c5c', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:44 PM',
        msg_type: 'membership',
        system_message: 'YT Member became a member!',
      },
    },
  ),
  base('m22', 'youtube', 'ytgifter', 'YT Gifter', '#ff7a7a', [],
    {
      tags: { 'msg-param-mass-gift-count': '5' },
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:44 PM',
        msg_type: 'membergift',
        system_message: 'YT Gifter gifted 5 memberships!',
      },
    },
  ),
  base('m23', 'youtube', 'ytsticker', 'YT Sticker', '#ff9e5c', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:45 PM',
        msg_type: 'supersticker',
        system_message: 'YT Sticker sent a $2.00 Super Sticker!',
      },
    },
  ),
  base('m24', 'kick', 'kicksub', 'Kick Sub', '#53fc18', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:45 PM',
        msg_type: 'sub',
        system_message: 'Kick Sub subscribed for 3 months!',
      },
    },
  ),
  base('m25', 'kick', 'kickgifter', 'Kick Gifter', '#7bff3c', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:46 PM',
        msg_type: 'kick_gift',
        system_message: 'Kick Gifter gifted 3 subs!',
      },
    },
  ),
  base('m26', 'kick', 'newkicker', 'New Kicker', '#a6f57a', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:46 PM',
        msg_type: 'kick_follow',
        system_message: 'New Kicker followed!',
      },
    },
  ),
  // A watch streak is a MILESTONE, not a subscription — distinct icon + wording.
  // The tags mirror the real USERNOTICE so the fire glyph, the app's phrasing,
  // and the +points chip all preview; the viewer's share message is the body.
  base('m12', 'twitch', 'streaker', 'Streaker', '#ffd166',
    [t('still no idea what game this is')],
    {
      tags: { 'msg-id': 'viewermilestone', 'msg-param-category': 'watch-streak', 'msg-param-value': '20', 'msg-param-copoReward': '450' },
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:46 PM',
        msg_type: 'viewermilestone',
        system_message: 'Streaker watched 20 streams in a row and still has no idea what game this is',
      },
    },
  ),
  base('m13', 'twitch', 'raidertv', 'RaiderTV', '#67c2ff', [],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:47 PM',
        msg_type: 'raid',
        system_message: '15 raiders from RaiderTV showed up to lurk in silence',
      },
    },
  ),
  // A community gift bomb: the "gifting N subs" announcement plus one individual
  // gift that shares its origin id. The overlay collapses these to just the
  // announcement (m20 is dropped), so a 20-sub bomb is one row, not 21.
  base('m19', 'twitch', 'vax1', 'Vax1', '#ff6a3c', [],
    {
      tags: { 'msg-id': 'submysterygift', 'msg-param-origin-id': 'bomb-vax1', 'msg-param-mass-gift-count': '20', 'msg-param-sub-plan': '1000' },
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:48 PM',
        msg_type: 'submysterygift',
        system_message: "Vax1 is gifting 20 Tier 1 Subs to the community! They've gifted a total of 120 in the channel!",
      },
    },
  ),
  base('m20', 'twitch', 'vax1', 'Vax1', '#ff6a3c', [],
    {
      tags: { 'msg-id': 'subgift', 'msg-param-origin-id': 'bomb-vax1' },
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:48 PM',
        msg_type: 'subgift',
        system_message: 'Vax1 gifted a Tier 1 sub to SylentVox!',
      },
    },
  ),
  // A first-time chatter, so the "First-time chatters" outline previews.
  base('m27', 'twitch', 'freshlurker', 'FreshLurker', '#5cc8ff',
    [t('first message ever, no pressure')],
    {
      tags: { 'first-msg': '1' },
      metadata: { is_action: false, is_mentioned: false, is_first_message: true, is_from_shared_chat: false, formatted_timestamp: '9:49 PM' },
    },
  ),
  // A few more normal chats so a tall overlay preview fills instead of looking sparse.
  base('m14', 'twitch', 'vex', 'Vex', '#9be7a3', [t('this overlay is cleaner than my browser history')]),
  base('m15', 'kick', 'kicker', 'Kicker', '#53fc18', [t('touching grass is a myth spread by big outdoor'), e('Kappa', '25')]),
  base('m16', 'youtube', 'ytchat', 'YT Chat', '#ff8a8a', [t('commenting so the algorithm blesses this stream')]),
  base('m17', 'twitch', 'poggers', 'Poggers', '#c792ea', [e('PogChamp', '305954156'), t('W play, framing this and putting it on my wall')]),
  // A gigantified emote (bits power-up): the last emote renders at 4x below the
  // line, so the Giant emotes toggle previews.
  base('m28', 'twitch', 'gigachadd', 'GigaChadd', '#ff7f50',
    [t('this moment deserves the big one'), e('PogChamp', '305954156')],
    { tags: { 'msg-id': 'gigantified-emote-message' } },
  ),
  // Emote-only gigantify: with Inline placement the 4x emote lands right after the
  // name, which is the layout the placement control exists to offer.
  base('m29', 'twitch', 'bigmood', 'BigMood', '#8b5cf6',
    [e('PogChamp', '305954156')],
    { tags: { 'msg-id': 'gigantified-emote-message' } },
  ),
  // A bits cheer. It is an ordinary chat message carrying a bit count, NOT a
  // USERNOTICE, which is why the Bits messages control exists: 'message' renders this
  // row inline, 'event' promotes it to the cheer card. The channel-prefix cheermote is
  // deliberate — those are the ones that used to fall through as plain text.
  base('m30', 'twitch', 'bitsbaron', 'BitsBaron', '#f6c445',
    [cm('mathox1Cheer100', 'mathox1cheer', 100, '100', '#9c3ee8'), t(' take my money, this play was unreal')],
    {
      metadata: {
        is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false,
        formatted_timestamp: '9:47 PM',
        bits_amount: 100,
      },
    },
  ),
  base('m18', 'tiktok', 'ttchat', 'TT Chat', '#00f2ea', [t('showed up, said hi, immediately leaving')]),
];

// ── Flowing preview ────────────────────────────────────────────────────────
// A pool of believable cross-platform chatters and lines, plus a generator that
// emits one random message. The builder's "Flow" toggle appends these on a timer
// so the preview reads like a live chat. Preview-only — the published overlay
// streams real chat; this never ships to a viewer.

interface FlowChatter {
  provider: ProviderId;
  username: string;
  display: string;
  color: string;
  paint?: OverlayPaint;
  badges?: OverlayMessage['badges'];
  streamNookUserNumber?: number;
  streamNookBadgeUrl?: string;
  atmosphere?: OverlayAtmosphere;
  avatar?: string;
}

const MOD_BADGE = { name: 'moderator', version: '1', image_url_4x: TWITCH_BADGE('3267646d-33f0-4b17-b3df-f923a41db1d0'), title: 'Moderator' };
const PRIME_BADGE = { name: 'premium', version: '1', image_url_4x: TWITCH_BADGE('bbbe0db0-a598-423e-86d0-f9fb98ca1933'), title: 'Prime Gaming' };
const VIP_BADGE = { name: 'vip', version: '1', image_url_4x: TWITCH_BADGE('b817aba4-fad8-49e2-b88a-7cc744dfa6ec'), title: 'VIP' };

// A spread of pure-CSS 7TV-style paints so painted chatters look varied, the way
// a real chat's cosmetics do. CSS gradients (no external asset) always render.
const PAINT_TOXIC = linearPaint('paint-toxic', 'Toxic', 90, [[0, '#b6ff00'], [1, '#00b36b']]);
const PAINT_MIAMI = linearPaint('paint-miami', 'Miami Nights', 110, [[0, '#0affed'], [0.5, '#8a5cff'], [1, '#ff5ca8']]);
const PAINT_GOLD = linearPaint('paint-gold', '24k', 100, [[0, '#fff1a8'], [0.5, '#ffd24a'], [1, '#b8860b']], ['#3a2a00', 0, 1, 2]);
const PAINT_SUNSET = linearPaint('paint-sunset', 'Sunset', 80, [[0, '#ff9a5c'], [0.5, '#ff4d6d'], [1, '#7a3cff']]);
const PAINT_ICE = linearPaint('paint-ice', 'Glacier', 115, [[0, '#e0fbff'], [0.5, '#8fd8ff'], [1, '#4aa8ff']], ['#02233a', 0, 1, 2]);
const PAINT_GALAXY = linearPaint('paint-galaxy', 'Galaxy', 125, [[0, '#5b2a86'], [0.4, '#8a5cff'], [0.7, '#3a86ff'], [1, '#00d4ff']], ['#05010f', 0, 1, 3]);

const FLOW_CHATTERS: FlowChatter[] = [
  { provider: 'twitch', username: 'pixel_gremlin', display: 'pixel_gremlin', color: '#ff4f8b', paint: PAINT_MIAMI },
  { provider: 'twitch', username: 'coffee_overflow', display: 'coffee_overflow', color: '#1e90ff', badges: [PRIME_BADGE] },
  { provider: 'twitch', username: 'certified_lurker', display: 'certified_lurker', color: '#8a2be2' },
  { provider: 'twitch', username: 'midlaner_diff', display: 'midlaner_diff', color: '#00c8a0', paint: PAINT_TOXIC },
  { provider: 'twitch', username: 'buffering99', display: 'buffering99', color: '#daa520' },
  { provider: 'twitch', username: 'notabot_promise', display: 'notabot_promise', color: '#57c2a3' },
  { provider: 'twitch', username: 'clipit_mods', display: 'ClipItMods', color: '#00b7ff', badges: [MOD_BADGE], paint: PAINT_ICE },
  { provider: 'twitch', username: 'backseat_gamer', display: 'backseat_gamer', color: '#ff6347', paint: PAINT_SUNSET },
  { provider: 'twitch', username: 'ratio_king', display: 'ratio_king', color: '#b22222', badges: [VIP_BADGE], paint: PAINT_GOLD },
  { provider: 'twitch', username: 'sleepy_panda', display: 'sleepy_panda', color: '#7fb2ff' },
  { provider: 'twitch', username: 'glassenjoyer', display: 'glassenjoyer', color: '#a98bff',
    paint: PAINT_SN, streamNookUserNumber: 128, streamNookBadgeUrl: 'https://streamnook.app/cosmetics/streamnook-badge-gold.png' },
  { provider: 'twitch', username: 'firstchair', display: 'firstchair', color: '#ffd166', paint: PAINT_EMBER },
  { provider: 'twitch', username: 'void_walker', display: 'void_walker', color: '#c792ea', paint: PAINT_GALAXY },
  { provider: 'twitch', username: 'nebula_ndy', display: 'nebula_ndy', color: '#8a5cff', paint: PAINT_NEBULA },
  { provider: 'youtube', username: 'notifgang', display: 'notif gang', color: '#ff5c5c',
    avatar: 'https://static-cdn.jtvnw.net/jtv_user_pictures/xarth/404_user_70x70.png' },
  { provider: 'youtube', username: 'superchat_stan', display: 'Superchat Stan', color: '#ff7a7a' },
  { provider: 'youtube', username: 'member_since', display: 'member since 2019', color: '#ff8a8a' },
  { provider: 'youtube', username: 'algo_enjoyer', display: 'Algorithm Enjoyer', color: '#ff9e5c' },
  { provider: 'kick', username: 'green_screen', display: 'green_screen', color: '#53fc18' },
  { provider: 'kick', username: 'degen_dave', display: 'degen_dave', color: '#7bff3c' },
  { provider: 'kick', username: 'convert_kev', display: 'convert_kev', color: '#a6f57a' },
  { provider: 'tiktok', username: 'fyp_wanderer', display: 'fyp wanderer', color: '#00f2ea' },
  { provider: 'tiktok', username: 'heart_spammer', display: 'heart spammer', color: '#25f4ee' },
  { provider: 'tiktok', username: 'ohno_notagain', display: 'ohno notagain', color: '#69c9d0' },
];

const FLOW_LINES = [
  'first', "who's winning", 'chat is this real', 'L + ratio', 'actually insane gameplay',
  'my goat', "he's so locked in", 'not the tutorial again', 'mods where', 'clip it',
  'GG', 'unlucky', 'we are so back', "it's so over", 'buffering on my end anyone else',
  'the music slaps what is this', 'hi from work', 'hi from class', '2 view gang',
  'notification squad', 'been here since 3 followers', 'touch grass? never heard of it',
  'why is chat like this', 'certified moment', 'he cooked', 'let him cook', 'sheesh',
  'no shot', 'did bro really', 'this the one', 'peak content', 'background noise while i work',
  'commenting for the algorithm', 'sending good vibes', 'W stream', 'based', 'real',
  'context?', 'brb feeding my cat', 'the goat has returned', 'average enjoyer',
];

// Real 7TV emotes (verified ids) rendered from the 7TV CDN — the same source the
// live overlay pulls, so the demo's emotes match what viewers actually use. A few
// Twitch globals round out the pool.
const sevenTv = (content: string, sid: string): MessageSegment => ({
  type: 'emote', content, emote_id: sid, emote_url: `https://cdn.7tv.app/emote/${sid}/2x.webp`,
});
const EMOTE_POOL: MessageSegment[] = [
  sevenTv('OMEGALUL', '01KWD232XK4CKE3XZF0Z4DM8YC'),
  sevenTv('ez', '01KVCS0WKKZDVTNFJBRGYDKQPK'),
  sevenTv('clap', '01KW5TZCRXY4KT2C1G09EVW4E0'),
  sevenTv('Catjam', '01KWK8F9TWGAJ8QP18WSK0XZGK'),
  sevenTv('PepeLaugh', '01KCYPEMZ8MNNS6Q12PJHY4149'),
  sevenTv('peepoHappy', '01KTR4A3Z08TPFNFA5CRVM9319'),
  sevenTv('Pog', '01KWVCCSQ3YYR6S1P7PH6MV3PD'),
  sevenTv('sadge', '01KVPQF71A6GTYHKFBWH5MQ0BY'),
  sevenTv('monkaS', '01KTFW0YRNDPZ6FATQG0CQZ6A7'),
  sevenTv('widepeepoHappy', '01KVXZ2KEZNCTMFTHM7PAP2X1B'),
  sevenTv('dinkdonk', '01KVP7645EWXBW3ZYNW8EJJ70H'),
  sevenTv('prayge', '01KVHZJ710B546YK7SST37DCYA'),
  sevenTv('peepoLeave', '01KJNXE25ZAZZ459F16CT4GEDJ'),
  sevenTv('AYAYA', '01KQXPJTS2RM5TX8K1VWFKETEX'),
  sevenTv('GigaChad', '01KWAMYZ68EA04PVD2ZQ2F37VH'),
  e('Kappa', '25'), e('PogChamp', '305954156'), e('LUL', '425618'),
];

// Global Twitch badges (verified UUIDs on the badge CDN) — the ones any account can
// carry regardless of channel: Turbo, Verified/Partner, GLHF Pledge, cheer tiers.
const GLOBAL_TWITCH_BADGES: NonNullable<OverlayMessage['badges']> = [
  { name: 'turbo', version: '1', image_url_4x: TWITCH_BADGE('bd444ec6-8f34-4bf9-91f4-af1e3428d80f'), title: 'Turbo' },
  { name: 'partner', version: '1', image_url_4x: TWITCH_BADGE('d12a2e27-16f6-41d0-ab77-b780518f00a3'), title: 'Verified' },
  { name: 'glhf-pledge', version: '1', image_url_4x: TWITCH_BADGE('3158e758-3cb4-43c5-94b3-7639810451c5'), title: 'GLHF Pledge' },
  { name: 'bits', version: '1', image_url_4x: TWITCH_BADGE('73b5c3fb-24f9-4a82-a852-2f475b59411c'), title: 'Bits' },
  { name: 'bits', version: '100', image_url_4x: TWITCH_BADGE('09d93036-e7ce-431c-9a9e-7044297133f2'), title: 'Bits (100)' },
  { name: 'bits', version: '1000', image_url_4x: TWITCH_BADGE('0d85a29e-79ad-4c63-a285-3acd2c66f2ba'), title: 'Bits (1K)' },
];

// Real 7TV badges (verified ids on the 7TV badge CDN), rendered as the message's
// 7TV badge — the cosmetic that sits by the name for 7TV users.
const sevenTvBadge = (bid: string) => `https://cdn.7tv.app/badge/${bid}/3x.webp`;
const SEVENTV_BADGES: Array<{ url: string; title: string }> = [
  { url: sevenTvBadge('01F8H53RZG000FJPFSJJHW714T'), title: '7TV Admin' },
  { url: sevenTvBadge('01F8H56KSR000FJPFSJJHW714W'), title: '7TV Moderator' },
  { url: sevenTvBadge('01F915ZNMR000B1B24Q19K3ZHB'), title: '7TV Contributor' },
  { url: sevenTvBadge('01G09ZZ6M000005RZWJQ2XQYEE'), title: '7TV Translator' },
  { url: sevenTvBadge('01FNXQY7D00005RKDHEQMRMQN1'), title: '7TV Subscriber' },
  { url: sevenTvBadge('01FNXRJNPG0005RKDHEQMRMQN3'), title: 'Subscriber - 6 Months' },
  { url: sevenTvBadge('01FNXRZDX00005RKDHEQMRMQN5'), title: 'Subscriber - 1 Year' },
];

// Stable hash so a given chatter's badge loadout is the SAME on every message
// (a user's cosmetics don't flicker between lines), while still varying per user.
const hashStr = (s: string): number => {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (Math.imul(h, 31) + s.charCodeAt(i)) >>> 0;
  return h;
};

const pick = <T,>(arr: T[]): T => arr[Math.floor(Math.random() * arr.length)];

// Cosmetics the chatter carries on a normal message (paint, badges, membership,
// avatar, plus a stable global-Twitch / 7TV badge loadout). Only sets `tags` when
// there's an avatar so `base`'s empty tags stand.
const chatterExtra = (c: FlowChatter): Partial<OverlayMessage> => {
  const extra: Partial<OverlayMessage> = {};
  if (c.paint) extra.paint = c.paint;
  if (c.streamNookUserNumber != null) extra.streamNookUserNumber = c.streamNookUserNumber;
  if (c.streamNookBadgeUrl) extra.streamNookBadgeUrl = c.streamNookBadgeUrl;
  if (c.atmosphere) extra.atmosphere = c.atmosphere;
  if (c.avatar) extra.tags = { avatar: c.avatar };

  const h = hashStr(c.username);
  const badges = c.badges ? [...c.badges] : [];
  // A global Twitch badge for paint-havers (always) and ~a third of other Twitch
  // users, so cosmetics-heavy accounts read like real chat.
  if (c.provider === 'twitch' && (c.paint || h % 3 === 0)) {
    badges.push(GLOBAL_TWITCH_BADGES[h % GLOBAL_TWITCH_BADGES.length]);
  }
  if (badges.length) extra.badges = badges;
  // A 7TV badge for ~half the paint-havers plus a few other Twitch users.
  if (c.provider === 'twitch' && ((c.paint && h % 2 === 0) || h % 5 === 0)) {
    const b = SEVENTV_BADGES[(h >>> 3) % SEVENTV_BADGES.length];
    extra.seventvBadgeUrl = b.url;
    extra.seventvBadgeTitle = b.title;
  }
  return extra;
};

// Real TikTok gifts with their real coin values and iconic look (Twemoji stand-in
// for the gift art, which lives on TikTok's own CDN). Cheap gifts get spammed in
// big combos; the expensive ones land solo — exactly how a TikTok stream reads.
const twemoji = (name: string, cp: string): MessageSegment => ({
  type: 'emote', content: name, emote_url: `https://cdn.jsdelivr.net/gh/jdecked/twemoji@15.1.0/assets/72x72/${cp}.png`,
});
const TIKTOK_GIFTS: Array<{ name: string; coins: number; cp: string; combos: number[] }> = [
  { name: 'Rose', coins: 1, cp: '1f339', combos: [1, 5, 10, 25, 99] },
  { name: 'Finger Heart', coins: 5, cp: '1f496', combos: [1, 3, 5, 10] },
  { name: 'Galaxy', coins: 1000, cp: '1f30c', combos: [1] },
  { name: 'Unicorn Fantasy', coins: 5000, cp: '1f984', combos: [1] },
  { name: 'Rocket', coins: 20000, cp: '1f680', combos: [1] },
  { name: 'Lion', coins: 29999, cp: '1f981', combos: [1] },
];

const money = (n: number) => `$${n.toFixed(2)}`;

// A provider-appropriate event with realistic values, or null to just send a chat
// line. Optional `segments` carry a gift's inline art (TikTok gifts).
interface FlowEvent { msg_type: string; system_message: string; tags?: Record<string, string>; segments?: MessageSegment[]; }
const eventFor = (c: FlowChatter): FlowEvent | null => {
  const months = pick([2, 3, 6, 9, 12, 24, 36]);
  if (c.provider === 'twitch') {
    const tier = pick(['1000', '1000', '1000', '2000', '3000']); // T1 most common
    const tierName = tier === '3000' ? 'Tier 3' : tier === '2000' ? 'Tier 2' : 'Tier 1';
    const bomb = pick([5, 5, 10, 20, 50, 100]);
    return pick<FlowEvent>([
      { msg_type: 'resub', system_message: `${c.display} subscribed at ${tierName}. They've subscribed for ${months} months!`,
        tags: { 'msg-param-sub-plan': tier, 'msg-param-cumulative-months': String(months) } },
      { msg_type: 'raid', system_message: `${pick([12, 34, 47, 156, 420, 892])} raiders from ${c.display} are here` },
      { msg_type: 'viewermilestone', system_message: `${c.display} is on a ${pick([5, 10, 25, 50, 100])}-stream watch streak` },
      { msg_type: 'submysterygift', system_message: `${c.display} is gifting ${bomb} ${tierName} Subs to the community!`,
        tags: { 'msg-id': 'submysterygift', 'msg-param-mass-gift-count': String(bomb), 'msg-param-sub-plan': tier } },
    ]);
  }
  if (c.provider === 'youtube') {
    const sc = pick([2, 5, 10, 20, 50, 100]);
    return pick<FlowEvent>([
      { msg_type: 'superchat', system_message: `${c.display} sent a ${money(sc)} Super Chat: ${pick(FLOW_LINES)}` },
      { msg_type: 'membership', system_message: months > 1 ? `${c.display} is a member for ${months} months!` : `${c.display} became a member!` },
      { msg_type: 'membergift', system_message: `${c.display} gifted ${pick([5, 10, 20])} memberships!`, tags: { 'msg-param-mass-gift-count': String(pick([5, 10, 20])) } },
      { msg_type: 'supersticker', system_message: `${c.display} sent a ${money(pick([2, 5, 10, 20]))} Super Sticker!` },
    ]);
  }
  if (c.provider === 'kick') {
    return pick<FlowEvent>([
      { msg_type: 'sub', system_message: `${c.display} subscribed for ${months} months!` },
      { msg_type: 'kick_gift', system_message: `${c.display} gifted ${pick([5, 10, 20, 50])} subs!` },
      { msg_type: 'kick_follow', system_message: `${c.display} followed!` },
    ]);
  }
  const g = pick(TIKTOK_GIFTS);
  const combo = pick(g.combos);
  return {
    msg_type: 'tiktok_gift',
    system_message: combo > 1 ? `${c.display} sent ${g.name} x${combo}` : `${c.display} sent ${g.name}`,
    segments: [twemoji(g.name, g.cp)],
  };
};

let flowSeq = 0;

/** One random preview message from a random chatter — mostly chat lines, roughly
 *  one in eight an event. Drives the builder's "Flow" demo. */
export function randomSampleMessage(): OverlayMessage {
  flowSeq += 1;
  const id = `flow-${flowSeq}-${Math.floor(Math.random() * 1e6)}`;
  const c = pick(FLOW_CHATTERS);
  const stamp = { formatted_timestamp: '', is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false };

  if (Math.random() < 0.12) {
    const ev = eventFor(c);
    if (ev) {
      return base(id, c.provider, c.username, c.display, c.color, ev.segments ?? [], {
        ...chatterExtra(c),
        ...(ev.tags ? { tags: ev.tags } : {}),
        metadata: { ...stamp, msg_type: ev.msg_type, system_message: ev.system_message },
      });
    }
  }

  // Normal chat: mostly a line (sometimes with a trailing emote), occasionally an
  // emote-only spam message — the real texture of a live chat.
  const segs: MessageSegment[] = [];
  if (Math.random() < 0.08) {
    const k = 1 + Math.floor(Math.random() * 3);
    for (let i = 0; i < k; i++) segs.push(pick(EMOTE_POOL));
  } else {
    segs.push(t(pick(FLOW_LINES)));
    if (Math.random() < 0.34) segs.push(pick(EMOTE_POOL));
  }
  return base(id, c.provider, c.username, c.display, c.color, segs, chatterExtra(c));
}

/** A small starter batch so the flow preview isn't empty on the first frame. */
export function seedFlowMessages(n = 8): OverlayMessage[] {
  return Array.from({ length: n }, () => randomSampleMessage());
}
