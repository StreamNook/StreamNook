// Realistic sample feed for the overlay builder preview. These mirror the exact
// BackendChatMessage shape the live overlay will receive over the WebSocket, so
// the preview renders through the same path as real data. Paints are pure-CSS
// 7TV-style gradients (no external asset, so they always render); emotes and
// badges are well-known global art on each platform's own CDN, so the demo never
// shows broken images. Every chatter and channel name here is made up.

import type { BackendChatMessage, MessageSegment } from '../../services/twitchChat';
import type { ProviderId } from '../../types/providers';
import type { SeventvEmoteChange } from './overlayConfig';

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
  /** `backdrop` is the colour a glyph badge is drawn on (see OverlayChat). */
  extraBadges?: { url: string; title?: string; source?: string; category?: string; backdrop?: string }[];
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
  /** A change to the channel's 7TV emote set: the row renders as a 7TV event
   *  instead of a chat message. */
  seventvEmoteChange?: SeventvEmoteChange;
};

type Badge = OverlayMessage['badges'][number];
type ExtraBadge = NonNullable<OverlayMessage['extraBadges']>[number];
type Metadata = NonNullable<OverlayMessage['metadata']>;

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

// A spread of pure-CSS 7TV-style paints so painted chatters look varied, the way
// a real chat's cosmetics do, plus the StreamNook-native look.
const PAINT_NEBULA = linearPaint('paint-nebula', 'Nebula', 120, [
  [0, '#8a5cff'], [0.5, '#ff5ca8'], [1, '#5cc8ff'],
], ['#000000', 0, 1, 2]);
const PAINT_EMBER = linearPaint('paint-ember', 'Ember', 90, [
  [0, '#ffd16a'], [0.6, '#ff7a3c'], [1, '#ff2e63'],
]);
const PAINT_SN = linearPaint('paint-sn', 'Winters Glass', 135, [
  [0, '#bfe3ff'], [0.5, '#7fb2ff'], [1, '#a98bff'],
], ['#0a1830', 0, 1, 3]);
const PAINT_TOXIC = linearPaint('paint-toxic', 'Toxic', 90, [[0, '#b6ff00'], [1, '#00b36b']]);
const PAINT_MIAMI = linearPaint('paint-miami', 'Miami Nights', 110, [[0, '#0affed'], [0.5, '#8a5cff'], [1, '#ff5ca8']]);
const PAINT_GOLD = linearPaint('paint-gold', '24k', 100, [[0, '#fff1a8'], [0.5, '#ffd24a'], [1, '#b8860b']], ['#3a2a00', 0, 1, 2]);
const PAINT_SUNSET = linearPaint('paint-sunset', 'Sunset', 80, [[0, '#ff9a5c'], [0.5, '#ff4d6d'], [1, '#7a3cff']]);
const PAINT_ICE = linearPaint('paint-ice', 'Glacier', 115, [[0, '#e0fbff'], [0.5, '#8fd8ff'], [1, '#4aa8ff']], ['#02233a', 0, 1, 2]);
const PAINT_GALAXY = linearPaint('paint-galaxy', 'Galaxy', 125, [[0, '#5b2a86'], [0.4, '#8a5cff'], [0.7, '#3a86ff'], [1, '#00d4ff']], ['#05010f', 0, 1, 3]);

// ── Badges ─────────────────────────────────────────────────────────────────
// Twitch's global badge art from its badge CDN. Channels upload their own
// subscriber art; these are Twitch's defaults for a channel without it.
const TWITCH_BADGE = (uuid: string) => `https://static-cdn.jtvnw.net/badges/v1/${uuid}/3`;
const badge = (name: string, version: string, uuid: string, title: string): Badge => ({
  name,
  version,
  image_url_4x: TWITCH_BADGE(uuid),
  title,
});

const BROADCASTER = badge('broadcaster', '1', '5527c58c-fb7d-422d-b71b-f309dcb85cc1', 'Broadcaster');
const MOD = badge('moderator', '1', '3267646d-33f0-4b17-b3df-f923a41db1d0', 'Moderator');
const VIP = badge('vip', '1', 'b817aba4-fad8-49e2-b88a-7cc744dfa6ec', 'VIP');
const PRIME = badge('premium', '1', 'bbbe0db0-a598-423e-86d0-f9fb98ca1933', 'Prime Gaming');
const TURBO = badge('turbo', '1', 'bd444ec6-8f34-4bf9-91f4-af1e3428d80f', 'Turbo');
const PARTNER = badge('partner', '1', 'd12a2e27-16f6-41d0-ab77-b780518f00a3', 'Verified');
const NO_AUDIO = badge('no_audio', '1', 'aef2cd08-f29b-45a1-8c12-d44d7fd5e6f0', 'Watching without audio');
const FOUNDER = badge('founder', '0', '511b78a9-ab37-472f-9569-457753bbe7d3', 'Founder');
const SUB_NEW = badge('subscriber', '0', '5d9f2208-5dd8-11e7-8513-2ff4adfae661', 'Subscriber');
const SUB_2 = badge('subscriber', '2', '25a03e36-2bb2-4625-bd37-d6d9d406238d', '2-Month Subscriber');
const SUB_3 = badge('subscriber', '3', 'e8984705-d091-4e54-8241-e53b30a84b0e', '3-Month Subscriber');
const SUB_6 = badge('subscriber', '6', 'ed51a614-2c44-4a60-80b6-62908436b43a', '6-Month Subscriber');
const SUB_9 = badge('subscriber', '9', 'b4e6b13a-a76f-4c56-87e1-9375a7aaa610', '9-Month Subscriber');
const PRED_BLUE_1 = badge('predictions', 'blue-1', 'e33d8b46-f63b-4e67-996d-4a7dcec0ad33', 'Predicted Blue (1)');
const PRED_PINK_2 = badge('predictions', 'pink-2', '4b76d5f2-91cc-4400-adf2-908a1e6cfd1e', 'Predicted Pink (2)');
const GIFTER_10 = badge('sub-gifter', '10', 'd333288c-65d7-4c7b-b691-cdd7b3484bf8', '10 Gift Subs');
const GIFTER_50 = badge('sub-gifter', '50', 'c4a29737-e8a5-4420-917a-314a447f083e', '50 Gift Subs');
const BITS_100 = badge('bits', '100', '09d93036-e7ce-431c-9a9e-7044297133f2', 'cheer 100');
const BITS_1000 = badge('bits', '1000', '0d85a29e-79ad-4c63-a285-3acd2c66f2ba', 'cheer 1000');
const BLOSSOM = badge('blossom-badge', '1', 'c255cb27-7e04-4be1-9dee-434e85523638', 'Blossom Badge Level 1');
const BLOOM = badge('bloom-badge', '1', '6c119c4d-adb3-4199-a0cb-3e09d7bc14c3', 'Bloom Badge Level 1');

// Third-party badges as the resolver tags them (`source` drives the per-service
// hide list). FFZ's /rounded art is the finished, coloured badge.
const FFZ_SUPPORTER: ExtraBadge = { url: 'https://cdn.frankerfacez.com/badge/3/4/rounded', title: 'FFZ Supporter', source: 'ffz' };
const CHATTERINO_SUPPORTER: ExtraBadge = { url: 'https://fourtf.com/chatterino/badges/supporter3x.png', title: 'Chatterino Supporter', source: 'chatterino' };

// Real 7TV badges (verified ids on the 7TV badge CDN), rendered as the message's
// 7TV badge — the cosmetic that sits by the name for 7TV users. Titles are the
// tooltips 7TV gives these badges (what the live feed shows), so none of them
// reads like a platform subscriber badge that failed to hide.
const sevenTvBadge = (bid: string) => `https://cdn.7tv.app/badge/${bid}/3x.webp`;
const SEVENTV_BADGES: Array<{ url: string; title: string }> = [
  { url: sevenTvBadge('01FNXQY7D00005RKDHEQMRMQN1'), title: '7TV Subscriber (1 Month)' },
  { url: sevenTvBadge('01FNXRJNPG0005RKDHEQMRMQN3'), title: '7TV Subscriber (6 Months)' },
  { url: sevenTvBadge('01FNXRZDX00005RKDHEQMRMQN5'), title: '7TV Subscriber (1 Year)' },
  { url: sevenTvBadge('01F915ZNMR000B1B24Q19K3ZHB'), title: '7TV Contributor' },
];
const seventv = (i: number): Partial<OverlayMessage> => ({
  seventvBadgeUrl: SEVENTV_BADGES[i].url,
  seventvBadgeTitle: SEVENTV_BADGES[i].title,
});

const SN_BADGE_GOLD = 'https://streamnook.app/cosmetics/streamnook-badge-gold.png';
const AVATAR = 'https://static-cdn.jtvnw.net/jtv_user_pictures/xarth/404_user_70x70.png';

// ── Emotes ─────────────────────────────────────────────────────────────────
// Well-known emotes on each provider's own CDN: Twitch globals, the BTTV and FFZ
// global sets, and the most-used 7TV uploads of each classic.
const t = (content: string): MessageSegment => ({ type: 'text', content });
const e = (content: string, id: string): MessageSegment => ({
  type: 'emote',
  content,
  emote_id: id,
  emote_url: `https://static-cdn.jtvnw.net/emoticons/v2/${id}/default/dark/3.0`,
});
const sevenTv = (content: string, id: string, zeroWidth = false): MessageSegment => ({
  type: 'emote',
  content,
  emote_id: id,
  emote_url: `https://cdn.7tv.app/emote/${id}/2x.webp`,
  ...(zeroWidth ? { is_zero_width: true } : null),
});
// BTTV and FFZ emotes carry no emote_id: on a failed load the renderer rebuilds
// the URL from the id as Twitch or 7TV art, which is the wrong CDN for these.
const bttv = (content: string, id: string): MessageSegment => ({
  type: 'emote', content, emote_url: `https://cdn.betterttv.net/emote/${id}/3x`,
});
const ffz = (content: string, id: string): MessageSegment => ({
  type: 'emote', content, emote_url: `https://cdn.frankerfacez.com/emote/${id}/4`,
});
// Twemoji stands in for TikTok's gift art, which lives on TikTok's own CDN.
const twemoji = (name: string, cp: string): MessageSegment => ({
  type: 'emote', content: name, emote_url: `https://cdn.jsdelivr.net/gh/jdecked/twemoji@15.1.0/assets/72x72/${cp}.png`,
});
const link = (content: string): MessageSegment => ({ type: 'link', content, url: `https://${content}` });

const KAPPA = e('Kappa', '25');
const LUL = e('LUL', '425618');
const SEEMS_GOOD = e('SeemsGood', '64138');
const NOT_LIKE_THIS = e('NotLikeThis', '58765');
const HEY_GUYS = e('HeyGuys', '30259');
const VOHIYO = e('VoHiYo', '81274');
const COOL_CAT = e('CoolCat', '58127');
const KREYGASM = e('Kreygasm', '41');
const HEART = e('<3', '555555584');
const GLITCH_CAT = e('GlitchCat', '304486301');

const PEEPO_HAPPY = sevenTv('peepoHappy', '01F6RC8C1G0003SBEQ3QZTEE99');
const PEEPO_CLAP = sevenTv('peepoClap', '01F6NET6G00009JYTB75QDKV1S');
const WIDE_PEEPO_HAPPY = sevenTv('widepeepoHappy', '01GF1Y2Q5G0000BGNJSP34TQRD');
const CLAP = sevenTv('Clap', '01F6NE9AER000CKKT9BSDYGT0J');
const PEPE_LAUGH = sevenTv('PepeLaugh', '01F6Q76NN80005589X3BDK9CN1');
const SADGE = sevenTv('Sadge', '01EZPG1FN80001SNAW00ADK2DY');
const EZ = sevenTv('EZ', '01F9FS6EEG0006XXD6DX0K9Y04');
const STARE = sevenTv('Stare', '01FA0WRBM0000AD7Y8T9ZJEDXQ');
const AWARE = sevenTv('Aware', '01FFWH9WV80000JT8GHDKHJNZC');
const HUH = sevenTv('HUH', '01FKKW7C1G0008TM5NY9QEFEDW');
const BEDGE = sevenTv('Bedge', '01F6MXJD8R000F76KNAAV5HDGD');
const OKAYGE = sevenTv('Okayge', '01F7GJ0N4R00074A83FVHRDMDB');
const GIGACHAD = sevenTv('GIGACHAD', '01F6MZGCNG000255K4X1K7NTHR');
const PRAYGE = sevenTv('Prayge', '01F6NACCD80006SZ7ZW5FMWKWK');
const LETSGO = sevenTv('LETSGO', '01F6RD7B88000B4N55W5NS55R7');
const ICANT = sevenTv('ICANT', '01FA0PBXM8000AD7Y8T9ZMTFZ5');
const OMEGALUL = sevenTv('OMEGALUL', '01F00Z3A9G0007E4VV006YKSK9');
// Zero-width overlays: each draws on top of the emote before it.
const RAIN_TIME = sevenTv('RainTime', '01FCY771D800007PQ2DF3GDTN6', true);
const PETPET = sevenTv('PETPET', '01FE3XY508000AA32JP519W2EW', true);

const MONKA_S = bttv('monkaS', '56e9f494fff3cc5c35e5287e');
const FEELS_GOOD_MAN = bttv('FeelsGoodMan', '566c9fde65dbbdab32ec053e');
const FEELS_BAD_MAN = bttv('FeelsBadMan', '566c9fc265dbbdab32ec053b');
const KKONA = bttv('KKona', '566ca04265dbbdab32ec054a');
const CAT_JAM = bttv('catJAM', '5f1b0186cf6d2144653d2970');

const CAT_BAG = ffz('CatBag', '25927');
const LILZ = ffz('LilZ', '28136');
const KEKW = ffz('KEKW', '381875');

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

// A Twitch chat GIF (Tier 2/3 subscribers): the bracketed description Twitch
// puts in the text plus the GIPHY asset from the `gifs` tag, taken verbatim
// from Twitch's documented example so the preview shows the real shape.
const SAMPLE_GIF: MessageSegment = {
  type: 'gif',
  content: '[Y A Y Yes GIF by Djemilah Birnie]',
  gif_id: 'joSNxeswxuc74Juo8X',
  gif_url: 'https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?cid=095d7a5dzizsiwgabonagkmigggv8v1spfai91ac3x0dsiy0&ep=v1_gifs_trending&rid=giphy.gif&ct=g',
};

/** A message body the way the tokenizer emits one: words and emotes in order,
 *  separated by single spaces that live in the text runs. */
const say = (...parts: Array<string | MessageSegment>): MessageSegment[] => {
  const out: MessageSegment[] = [];
  for (const part of parts) {
    const prev = out[out.length - 1];
    if (typeof part === 'string') {
      // Words join the text run before them, or open one after an emote.
      if (prev?.type === 'text') out[out.length - 1] = t(`${prev.content} ${part}`);
      else out.push(t(prev ? ` ${part}` : part));
    } else if (prev?.type === 'text') {
      out[out.length - 1] = t(`${prev.content} `);
      out.push(part);
    } else {
      if (prev) out.push(t(' '));
      out.push(part);
    }
  }
  return out;
};

const META: Metadata = { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, formatted_timestamp: '9:41 PM' };
const meta = (over: Partial<Metadata>): Metadata => ({ ...META, ...over });

// An event row: the platform's system message plus the tags it rides on.
const event = (msg_type: string, system_message: string, tags: Record<string, string> = {}): Partial<OverlayMessage> => ({
  tags,
  metadata: meta({ msg_type, system_message }),
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
  user_id: `uid-${username}`,
  timestamp: new Date(0).toISOString(),
  content: segments.map((s) => ('content' in s ? s.content : '')).join('').trim(),
  provider,
  channel: provider === 'twitch' ? 'harborlight' : `${provider}:harborlight`,
  badges: [],
  emotes: [],
  layout: { height: 0, width: 0 },
  tags: {},
  segments,
  metadata: META,
  ...extra,
});

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

// Clock times that tick forward down the list, so the timestamp toggle previews
// a believable run of times rather than one repeated minute.
const withClock = (list: OverlayMessage[]): OverlayMessage[] =>
  list.map((m, i) => ({
    ...m,
    metadata: { ...META, ...m.metadata, formatted_timestamp: `9:${String(18 + Math.floor(i / 2)).padStart(2, '0')} PM` },
  }));

// One evening of a Twitch-led chat with the other platforms merged in, oldest
// first. The renderer keeps the NEWEST rows that fit, so the last dozen or so are
// what the default 400 x 640 canvas shows: that tail carries every look the builder
// edits (badges of each kind, paints, a GIF, a reply, a first-time chatter, the
// event styles), spread the way a real busy chat spreads them. Rarer looks (the
// giant emote, YouTube avatars, Kick and YouTube events) sit higher up, where a
// taller canvas reaches them, and the builder brings one into view while its
// setting is edited (see SAMPLE_FOCUS).
export const SAMPLE_MESSAGES: OverlayMessage[] = withClock([
  // YouTube names arrive as @handles, so this previews the "@ before usernames"
  // toggle, and the avatar previews "Profile pictures".
  base('s-yt-hello', 'youtube', '@lofilena', '@LofiLena', '#ff8a8a',
    say('hi from the youtube side 👋'),
    { tags: { avatar: AVATAR } },
  ),
  base('s-kick-follow', 'kick', 'bagel_baron', 'bagel_baron', '#7bff3c', [],
    event('kick_follow', 'bagel_baron followed!'),
  ),
  base('s-pebble', 'twitch', 'pebble_pusher', 'pebble_pusher', '#2e8b57',
    say('chat is this the final boss', HUH),
    { paint: PAINT_TOXIC, badges: [SUB_2], ...seventv(0) },
  ),
  base('s-streamer-try', 'twitch', 'harborlight', 'Harborlight', '#e0457b',
    say('ok this is the one, i can feel it', PRAYGE),
    { paint: PAINT_NEBULA, badges: [BROADCASTER, PARTNER] },
  ),
  base('s-yt-member', 'youtube', '@cafecosmo', '@CafeCosmo', '#ff9e5c', [],
    event('membership', '@CafeCosmo became a member!'),
  ),
  base('s-dan', 'twitch', 'driftwood_dan', 'driftwood_dan', '#5f9ea0',
    say('he is so locked in'),
    { badges: [SUB_6, PRIME] },
  ),
  base('s-pepe', 'twitch', 'pepe_patrol', 'pepe_patrol', '#9acd32',
    say(MONKA_S, 'my heart cant take this'),
    { badges: [TURBO], ...seventv(3) },
  ),
  base('s-kick-gift', 'kick', 'grillmaster_kev', 'grillmaster_kev', '#53fc18', [],
    event('kick_gift', 'grillmaster_kev gifted 5 subs!', { 'msg-param-mass-gift-count': '5' }),
  ),
  base('s-tt-hello', 'tiktok', 'sunnysideup', 'sunny side up', '#25f4ee',
    say('hiii tiktok is here 💗'),
  ),
  // A gigantified emote (bits power-up): the last emote renders at 4x below the
  // line, so the Giant emotes toggle and its Placement preview.
  base('s-giant', 'twitch', 'loud_lettuce', 'loud_lettuce', '#ff7f50',
    say('HE DID IT', GLITCH_CAT),
    { badges: [SUB_6, BITS_100], tags: { 'msg-id': 'gigantified-emote-message' } },
  ),
  base('s-letsgo', 'twitch', 'tea_time_tom', 'tea_time_tom', '#2e8b57',
    say(LETSGO, LETSGO),
    { badges: [SUB_2] },
  ),
  // A watch streak is a MILESTONE, not a subscription: distinct icon + wording.
  // The tags mirror the real USERNOTICE so the fire glyph, the app's phrasing,
  // and the +points chip all preview; the viewer's share message is the body.
  base('s-streak', 'twitch', 'quietcomet', 'quietcomet', '#8a2be2',
    say('here every night lol'),
    {
      badges: [SUB_3],
      ...event('viewermilestone', 'quietcomet watched 12 consecutive streams this month and sparked a watch streak!', {
        'msg-id': 'viewermilestone', 'msg-param-category': 'watch-streak', 'msg-param-value': '12', 'msg-param-copoReward': '450',
      }),
    },
  ),
  // A zero-width 7TV emote (PETPET) sits on the emote before it, and the navy
  // name shows Readable name colors at work.
  base('s-petpet', 'twitch', 'rainy_ren', 'rainy_ren', '#1a237e',
    say('he earned this', LILZ, PETPET),
    { badges: [PRIME] },
  ),
  base('s-kick-sub', 'kick', 'bagel_baron', 'bagel_baron', '#7bff3c', [],
    event('sub', 'bagel_baron subscribed for 3 months!'),
  ),
  base('s-yt-sticker', 'youtube', '@lofilena', '@LofiLena', '#ff8a8a', [],
    event('supersticker', '@LofiLena sent a $2.00 Super Sticker!'),
  ),
  base('s-yt-clean', 'youtube', '@membersince2019', '@membersince2019', '#ff7a7a',
    say('that was so clean'),
    { tags: { avatar: AVATAR } },
  ),
  base('s-yt-gift', 'youtube', '@cafecosmo', '@CafeCosmo', '#ff9e5c', [],
    event('membergift', '@CafeCosmo gifted 5 memberships!', { 'msg-param-mass-gift-count': '5' }),
  ),
  base('s-raid', 'twitch', 'cozykettle', 'CozyKettle', '#ff69b4', [],
    {
      badges: [PARTNER],
      ...event('raid', '38 raiders from CozyKettle have joined!', {
        'msg-id': 'raid', 'msg-param-viewerCount': '38', 'msg-param-displayName': 'CozyKettle', 'msg-param-login': 'cozykettle',
      }),
    },
  ),
  base('s-raider', 'twitch', 'kettle_crew_kai', 'kettle_crew_kai', '#00c8a0',
    say('kettle crew says hi', PEEPO_CLAP),
    { badges: [FOUNDER], extraBadges: [CHATTERINO_SUPPORTER] },
  ),
  base('s-lol', 'twitch', 'popcorn_pete', 'popcorn_pete', '#d2691e',
    say(LUL, LUL),
    { badges: [NO_AUDIO] },
  ),
  // ── The visible tail at the default canvas starts about here ──
  base('s-nia', 'twitch', 'night_owl_nia', 'night_owl_nia', '#c792ea',
    say('wait what did i miss'),
  ),
  base('s-superchat', 'youtube', '@miramoon', '@MiraMoon', '#ff5c5c', [],
    event('superchat', '@MiraMoon sent a $5.00 Super Chat: gg!!'),
  ),
  // A Twitch reply: 'full' draws the parent above the message, 'mention'
  // prefixes the body with the parent's @name.
  base('s-reply', 'twitch', 'mossy_bricks', 'mossy_bricks', '#1e90ff',
    say('he beat it first try', KEKW),
    {
      badges: [SUB_6],
      extraBadges: [FFZ_SUPPORTER],
      metadata: meta({
        reply_info: {
          parent_msg_id: 's-nia', parent_display_name: 'night_owl_nia', parent_user_id: 'uid-night_owl_nia',
          parent_user_login: 'night_owl_nia', parent_msg_body: 'wait what did i miss',
        },
      }),
    },
  ),
  // A first-time chatter, so the "First-time chatters" outline previews.
  base('s-first', 'twitch', 'saltwater_sam', 'saltwater_sam', '#daa520',
    say('hi! first stream i caught live'),
    { tags: { 'first-msg': '1' }, metadata: meta({ is_first_message: true }) },
  ),
  base('s-resub', 'twitch', 'bram_jam', 'bram_jam', '#ff4500',
    say('still skips every chest', NOT_LIKE_THIS),
    {
      badges: [SUB_9],
      ...event('resub', "bram_jam subscribed at Tier 1. They've subscribed for 14 months!", {
        'msg-id': 'resub', 'msg-param-sub-plan': '1000', 'msg-param-cumulative-months': '14',
      }),
    },
  ),
  // A Twitch chat GIF from a Tier 2 subscriber.
  base('s-gif', 'twitch', 'confetti_cat', 'confetti_cat', '#ff69b4',
    say('FINALLY', SAMPLE_GIF),
    { badges: [SUB_3], ...seventv(1) },
  ),
  base('s-predict', 'twitch', 'ratio_rhea', 'ratio_rhea', '#b22222',
    say('pink believers eating', EZ),
    { paint: PAINT_EMBER, badges: [VIP, PRED_PINK_2] },
  ),
  base('s-kick', 'kick', 'grillmaster_kev', 'grillmaster_kev', '#53fc18',
    say('W'),
  ),
  // A bits cheer. It is an ordinary chat message carrying a bit count, NOT a
  // USERNOTICE, which is why the Bits messages control exists: 'message' renders
  // this row inline, 'event' promotes it to the cheer card.
  base('s-cheer', 'twitch', 'bits_and_bobs', 'bits_and_bobs', '#f6c445',
    say(cm('Cheer100', 'cheer', 100, '100', '#9c3ee8'), 'clutch tax', CAT_JAM),
    { badges: [SUB_2, BITS_1000, BLOOM], metadata: meta({ bits_amount: 100 }) },
  ),
  base('s-mod', 'twitch', 'marlowe', 'Marlowe', '#00b7ff',
    say(link('clips.twitch.tv/BraveOtterJump')),
    { paint: PAINT_ICE, badges: [MOD, SUB_9] },
  ),
  // A community gift bomb: the "gifting N subs" announcement plus one individual
  // gift that shares its origin id. The overlay collapses these to just the
  // announcement (the single gift is dropped), so a 5-sub bomb is one row.
  base('s-giftbomb', 'twitch', 'tiny_tornado', 'tiny_tornado', '#ff7f50', [],
    {
      badges: [SUB_6, GIFTER_50],
      ...event('submysterygift', 'tiny_tornado is gifting 5 Tier 1 Subs to the community!', {
        'msg-id': 'submysterygift', 'msg-param-origin-id': 'bomb-tiny-tornado', 'msg-param-mass-gift-count': '5',
        'msg-param-sender-count': '52', 'msg-param-sub-plan': '1000',
      }),
    },
  ),
  base('s-giftbomb-1', 'twitch', 'tiny_tornado', 'tiny_tornado', '#ff7f50', [],
    event('subgift', 'tiny_tornado gifted a Tier 1 sub to quiet_quill!', {
      'msg-id': 'subgift', 'msg-param-origin-id': 'bomb-tiny-tornado', 'msg-param-recipient-display-name': 'quiet_quill',
    }),
  ),
  // Cross-platform events speak the same vocabulary, so a TikTok gift renders
  // through the same event row, with the gift's art inline.
  base('s-tt-gift', 'tiktok', 'sunnysideup', 'sunny side up', '#25f4ee', [twemoji('Rose', '1f339')],
    event('tiktok_gift', 'sunny side up sent Rose x5'),
  ),
  // A StreamNook member: the member badge, the Winters Glass paint and the
  // Aurora atmosphere behind the row. The @mention is plain text in chat.
  base('s-member', 'twitch', 'nightglass', 'nightglass', '#a98bff',
    say('@Harborlight hydrate', PEEPO_HAPPY, RAIN_TIME),
    {
      paint: PAINT_SN,
      streamNookUserNumber: 42,
      streamNookBadgeUrl: SN_BADGE_GOLD,
      atmosphere: SAMPLE_ATMOSPHERE,
      badges: [SUB_3, BLOSSOM],
    },
  ),
  // A channel-point redemption with no text. The feed names the reward in tags,
  // so the card, the points glyph and the cost all preview.
  base('s-reward', 'twitch', 'fernlight', 'Fernlight', '#4fc3f7', [],
    { tags: { 'sn-reward-title': 'Hydrate', 'sn-reward-cost': '5000' } },
  ),
]);

/** Sample ids a setting needs on screen to preview, for settings whose example
 *  sits above the default canvas's visible tail. */
const SAMPLE_FOCUS: Partial<Record<string, string>> = {
  giantEmotes: 's-giant',
  giantEmoteAlign: 's-giant',
  showAvatars: 's-yt-hello',
  showAtSign: 's-yt-hello',
};

/** SAMPLE_MESSAGES with the example for `settingKey` moved to the newest slot,
 *  so editing that setting shows its effect on any canvas. The same array when
 *  the setting's example is already in view. */
export function samplesFocusing(settingKey: string | null): OverlayMessage[] {
  const id = settingKey ? SAMPLE_FOCUS[settingKey] : undefined;
  const focus = id ? SAMPLE_MESSAGES.find((m) => m.id === id) : undefined;
  if (!focus) return SAMPLE_MESSAGES;
  return [...SAMPLE_MESSAGES.filter((m) => m !== focus), focus];
}

/** Whether `settingKey` has an example that samplesFocusing brings into view. */
export const hasSampleFocus = (settingKey: string): boolean => settingKey in SAMPLE_FOCUS;

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
  extraBadges?: ExtraBadge[];
  streamNookUserNumber?: number;
  streamNookBadgeUrl?: string;
  atmosphere?: OverlayAtmosphere;
  avatar?: string;
}

const FLOW_CHATTERS: FlowChatter[] = [
  { provider: 'twitch', username: 'pixel_gremlin', display: 'pixel_gremlin', color: '#ff4f8b', paint: PAINT_MIAMI, badges: [SUB_6] },
  { provider: 'twitch', username: 'coffee_overflow', display: 'coffee_overflow', color: '#1e90ff', badges: [SUB_3, PRIME] },
  { provider: 'twitch', username: 'certified_lurker', display: 'certified_lurker', color: '#8a2be2', badges: [NO_AUDIO] },
  { provider: 'twitch', username: 'midlaner_diff', display: 'midlaner_diff', color: '#00c8a0', paint: PAINT_TOXIC, badges: [SUB_9, PRED_BLUE_1] },
  { provider: 'twitch', username: 'buffering99', display: 'buffering99', color: '#daa520' },
  { provider: 'twitch', username: 'notabot_promise', display: 'notabot_promise', color: '#57c2a3', badges: [SUB_NEW] },
  { provider: 'twitch', username: 'clipit_mods', display: 'ClipItMods', color: '#00b7ff', badges: [MOD, SUB_9], paint: PAINT_ICE },
  { provider: 'twitch', username: 'backseat_gamer', display: 'backseat_gamer', color: '#ff6347', paint: PAINT_SUNSET, badges: [PRED_PINK_2] },
  { provider: 'twitch', username: 'loud_lettuce', display: 'loud_lettuce', color: '#ff7f50', badges: [SUB_6, BITS_100] },
  { provider: 'twitch', username: 'sleepy_panda', display: 'sleepy_panda', color: '#7fb2ff', badges: [FOUNDER], extraBadges: [FFZ_SUPPORTER] },
  { provider: 'twitch', username: 'glassenjoyer', display: 'glassenjoyer', color: '#a98bff', badges: [SUB_3],
    paint: PAINT_SN, streamNookUserNumber: 128, streamNookBadgeUrl: SN_BADGE_GOLD },
  { provider: 'twitch', username: 'firstchair', display: 'firstchair', color: '#ffd166', paint: PAINT_EMBER, badges: [SUB_2, GIFTER_10] },
  { provider: 'twitch', username: 'void_walker', display: 'void_walker', color: '#c792ea', paint: PAINT_GALAXY, badges: [BITS_1000] },
  { provider: 'twitch', username: 'gold_standard', display: 'gold_standard', color: '#b22222', badges: [VIP, SUB_6], paint: PAINT_GOLD },
  { provider: 'twitch', username: 'nebula_ndy', display: 'nebula_ndy', color: '#8a5cff', paint: PAINT_NEBULA, badges: [TURBO], extraBadges: [CHATTERINO_SUPPORTER] },
  { provider: 'twitch', username: 'tea_time_tom', display: 'tea_time_tom', color: '#2e8b57', badges: [SUB_2, BLOSSOM] },
  { provider: 'twitch', username: 'quietcomet', display: 'quietcomet', color: '#9acd32', badges: [BLOOM] },
  { provider: 'youtube', username: '@notifgang', display: '@notifgang', color: '#ff5c5c', avatar: AVATAR },
  { provider: 'youtube', username: '@superchatstan', display: '@SuperchatStan', color: '#ff7a7a' },
  { provider: 'youtube', username: '@membersince2019', display: '@membersince2019', color: '#ff8a8a', avatar: AVATAR },
  { provider: 'kick', username: 'green_screen', display: 'green_screen', color: '#53fc18' },
  { provider: 'kick', username: 'degen_dave', display: 'degen_dave', color: '#7bff3c' },
  { provider: 'tiktok', username: 'fyp_wanderer', display: 'fyp wanderer', color: '#00f2ea' },
  { provider: 'tiktok', username: 'heart_spammer', display: 'heart spammer', color: '#25f4ee' },
];

// Mostly short, the way a busy chat actually reads.
const FLOW_LINES = [
  'W', 'GG', 'LMAO', 'true', 'clean', 'no shot', 'real', 'lol', 'NO WAY', 'wait what',
  "who's winning", 'chat is this real', 'actually insane', "he's so locked in", 'mods where',
  'clip it', 'unlucky', 'we are so back', "it's so over", 'run it back', 'one more',
  'the music slaps what is this', 'hi from work', 'hi from class', 'how is he this good',
  'that timing though', 'he cooked', 'let him cook', 'sheesh', 'did bro really', 'this the one',
  'background noise while i work', 'chat be nice', 'context?', 'brb feeding my cat', 'i was here',
  'how long has he been live', 'song name?', 'that was so close', 'gn chat', 'hydrate',
];

// Replies and @mentions answer someone else, so they read short.
const ANSWERS = ['true', 'you called it', 'no shot', 'lol same', 'fr', 'hi!', 'it was the first try', 'its in the panels'];
const FIRST_LINES = ['hi! first time here', 'just found this stream', 'hello from the front page', 'first stream i caught live'];
const LINKS = ['clips.twitch.tv/BravePlumpWombat', 'clips.twitch.tv/TinySleepyFox', 'clips.twitch.tv/LoudCrispyOtter'];
const EMOJI = ['😭', '💀', '🔥', '😂', '👀', '❤️', '🙏'];

const TWITCH_EMOTES: MessageSegment[] = [
  KAPPA, LUL, SEEMS_GOOD, NOT_LIKE_THIS, HEY_GUYS, VOHIYO, COOL_CAT, KREYGASM, HEART,
  MONKA_S, FEELS_GOOD_MAN, FEELS_BAD_MAN, KKONA, CAT_JAM,
  CAT_BAG, LILZ, KEKW,
];
// 7TV emotes also reach Kick and YouTube chat through the 7TV extension.
const SEVENTV_EMOTES: MessageSegment[] = [
  PEEPO_HAPPY, PEEPO_CLAP, WIDE_PEEPO_HAPPY, CLAP, PEPE_LAUGH, SADGE, EZ, STARE, AWARE, HUH,
  BEDGE, OKAYGE, GIGACHAD, PRAYGE, LETSGO, ICANT, OMEGALUL,
];
const ZERO_WIDTH: MessageSegment[] = [RAIN_TIME, PETPET];
// Base emotes a zero-width overlay reads well on.
const ZW_BASES: MessageSegment[] = [PEEPO_HAPPY, OKAYGE, GIGACHAD, CAT_BAG, COOL_CAT];

// Stable hash so a given chatter's badge loadout is the SAME on every message
// (a user's cosmetics don't flicker between lines), while still varying per user.
const hashStr = (s: string): number => {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (Math.imul(h, 31) + s.charCodeAt(i)) >>> 0;
  return h;
};

const pick = <T,>(arr: T[]): T => arr[Math.floor(Math.random() * arr.length)];

// Cosmetics the chatter carries on a normal message (paint, badges, membership,
// avatar, plus a stable 7TV badge for about a third of Twitch chatters). Only
// sets `tags` when there's an avatar so `base`'s empty tags stand.
const chatterExtra = (c: FlowChatter): Partial<OverlayMessage> => {
  const extra: Partial<OverlayMessage> = {};
  if (c.paint) extra.paint = c.paint;
  if (c.badges?.length) extra.badges = c.badges;
  if (c.extraBadges?.length) extra.extraBadges = c.extraBadges;
  if (c.streamNookUserNumber != null) extra.streamNookUserNumber = c.streamNookUserNumber;
  if (c.streamNookBadgeUrl) extra.streamNookBadgeUrl = c.streamNookBadgeUrl;
  if (c.atmosphere) extra.atmosphere = c.atmosphere;
  if (c.avatar) extra.tags = { avatar: c.avatar };
  const h = hashStr(c.username);
  if (c.provider === 'twitch' && h % 3 === 0) {
    const b = SEVENTV_BADGES[(h >>> 3) % SEVENTV_BADGES.length];
    extra.seventvBadgeUrl = b.url;
    extra.seventvBadgeTitle = b.title;
  }
  return extra;
};

// Real TikTok gifts with their real coin values and iconic look (Twemoji stand-in
// for the gift art). Cheap gifts get spammed in big combos; the expensive ones
// land solo, exactly how a TikTok stream reads.
const TIKTOK_GIFTS: Array<{ name: string; coins: number; cp: string; combos: number[] }> = [
  { name: 'Rose', coins: 1, cp: '1f339', combos: [1, 5, 10, 25, 99] },
  { name: 'Finger Heart', coins: 5, cp: '1f496', combos: [1, 3, 5, 10] },
  { name: 'Galaxy', coins: 1000, cp: '1f30c', combos: [1] },
  { name: 'Unicorn Fantasy', coins: 5000, cp: '1f984', combos: [1] },
  { name: 'Rocket', coins: 20000, cp: '1f680', combos: [1] },
  { name: 'Lion', coins: 29999, cp: '1f981', combos: [1] },
];

// Twitch's cheer tiers: the gem and the colour of the count both follow the amount.
const CHEER_TIERS: Array<{ min: number; tier: string; color: string }> = [
  { min: 10000, tier: '10000', color: '#f43021' },
  { min: 5000, tier: '5000', color: '#0099fe' },
  { min: 1000, tier: '1000', color: '#1db2a5' },
  { min: 100, tier: '100', color: '#9c3ee8' },
  { min: 1, tier: '1', color: '#979797' },
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
    const bomb = pick([5, 5, 10, 20, 50]);
    const raiders = pick([12, 34, 47, 156, 420]);
    return pick<FlowEvent>([
      { msg_type: 'resub', system_message: `${c.display} subscribed at ${tierName}. They've subscribed for ${months} months!`,
        tags: { 'msg-param-sub-plan': tier, 'msg-param-cumulative-months': String(months) } },
      { msg_type: 'raid', system_message: `${raiders} raiders from ${c.display} have joined!`,
        tags: { 'msg-param-viewerCount': String(raiders), 'msg-param-displayName': c.display } },
      { msg_type: 'submysterygift', system_message: `${c.display} is gifting ${bomb} ${tierName} Subs to the community!`,
        tags: { 'msg-id': 'submysterygift', 'msg-param-mass-gift-count': String(bomb), 'msg-param-sub-plan': tier } },
      // A redemption carries no msg-id or system message; the reward tags make it one.
      { msg_type: '', system_message: '',
        tags: pick([
          { 'sn-reward-title': 'Hydrate', 'sn-reward-cost': '5000' },
          { 'sn-reward-title': 'Pick the next map', 'sn-reward-cost': '25000' },
          { 'sn-reward-title': 'Posture check', 'sn-reward-cost': '1000' },
        ]) },
    ]);
  }
  if (c.provider === 'youtube') {
    const sc = pick([2, 5, 10, 20, 50, 100]);
    const gifts = pick([5, 10, 20]);
    return pick<FlowEvent>([
      { msg_type: 'superchat', system_message: `${c.display} sent a ${money(sc)} Super Chat: ${pick(FLOW_LINES)}` },
      { msg_type: 'membership', system_message: months > 1 ? `${c.display} is a member for ${months} months!` : `${c.display} became a member!` },
      { msg_type: 'membergift', system_message: `${c.display} gifted ${gifts} memberships!`, tags: { 'msg-param-mass-gift-count': String(gifts) } },
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

// A chat line: mostly words, sometimes with a trailing emote (or emoji off
// Twitch), now and then emote-only spam or an emote wearing a zero-width overlay.
const flowLine = (c: FlowChatter): MessageSegment[] => {
  const emotes = c.provider === 'twitch' ? [...TWITCH_EMOTES, ...SEVENTV_EMOTES] : c.provider === 'tiktok' ? [] : SEVENTV_EMOTES;
  const roll = Math.random();
  if (emotes.length && roll < 0.08) {
    const em = pick(emotes);
    return say(...Array.from({ length: 1 + Math.floor(Math.random() * 3) }, () => em));
  }
  if (c.provider !== 'tiktok' && roll < 0.13) return say(pick(FLOW_LINES), pick(ZW_BASES), pick(ZERO_WIDTH));
  const line = pick(FLOW_LINES);
  if (emotes.length && roll < 0.45) return say(line, pick(emotes));
  if (roll < 0.55) return say(`${line} ${pick(EMOJI)}`);
  return say(line);
};

let flowSeq = 0;

// How often each kind of message turns up, as cumulative odds. Everything past
// the last bound is an ordinary chat line. The GIF and the giant emote are rare
// enough that most windows of the preview show neither.
const FLOW_ODDS: Array<[number, string]> = [
  [0.12, 'event'], [0.16, 'reply'], [0.19, 'mention'], [0.21, 'first'], [0.235, 'cheer'],
  [0.245, 'link'], [0.253, 'gif'], [0.261, 'giant'],
];
// Kinds only Twitch chat has. Elsewhere a reply is a plain @mention and the
// rest fall back to an ordinary line.
const TWITCH_ONLY = new Set(['reply', 'first', 'cheer', 'gif', 'giant']);

/** One random preview message from a random chatter: mostly chat lines, roughly
 *  one in eight an event, with the rarer looks (replies, mentions, first-time
 *  chatters, cheers, links, a GIF, a giant emote) sprinkled at about the rate a
 *  live chat shows them. Drives the builder's "Flow" demo. */
export function randomSampleMessage(): OverlayMessage {
  flowSeq += 1;
  const id = `flow-${flowSeq}-${Math.floor(Math.random() * 1e6)}`;
  const c = pick(FLOW_CHATTERS);
  const extra = chatterExtra(c);
  const stamp = { formatted_timestamp: '', is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false };
  const msg = (segs: MessageSegment[], over: Partial<OverlayMessage> = {}) =>
    base(id, c.provider, c.username, c.display, c.color, segs, { ...extra, metadata: stamp, ...over });

  const roll = Math.random();
  let kind = FLOW_ODDS.find(([bound]) => roll < bound)?.[1] ?? 'line';
  if (c.provider !== 'twitch' && TWITCH_ONLY.has(kind)) kind = kind === 'reply' ? 'mention' : 'line';
  if (c.provider === 'tiktok' && kind === 'link') kind = 'line';
  const others = FLOW_CHATTERS.filter((o) => o.provider === c.provider && o !== c);
  const other = pick(others.length ? others : FLOW_CHATTERS);

  switch (kind) {
    case 'event': {
      const ev = eventFor(c);
      if (!ev) break;
      return msg(ev.segments ?? [], {
        ...(ev.tags ? { tags: ev.tags } : {}),
        metadata: { ...stamp, msg_type: ev.msg_type, system_message: ev.system_message },
      });
    }
    case 'reply':
      return msg(say(pick(ANSWERS)), {
        metadata: {
          ...stamp,
          reply_info: {
            parent_msg_id: `flow-parent-${flowSeq}`, parent_display_name: other.display, parent_user_id: `uid-${other.username}`,
            parent_user_login: other.username, parent_msg_body: pick(FLOW_LINES),
          },
        },
      });
    case 'mention':
      return msg(say(`@${other.display.replace(/^@/, '')} ${pick(ANSWERS)}`));
    case 'first': {
      // A first-time chatter is someone new, so no badges or cosmetics.
      const name = pick(['hello_harper', 'tumbleweed_tia', 'just_browsing_joe', 'newbie_nova']);
      return base(id, 'twitch', name, name, '#5cc8ff', say(pick(FIRST_LINES)), {
        tags: { 'first-msg': '1' },
        metadata: { ...stamp, is_first_message: true },
      });
    }
    case 'cheer': {
      const bits = pick([100, 100, 250, 500, 1000, 5000]);
      const tier = CHEER_TIERS.find((x) => bits >= x.min)!;
      return msg(say(cm(`Cheer${bits}`, 'cheer', bits, tier.tier, tier.color), pick(['clutch tax', 'for the play', 'gg', 'hydrate'])), {
        metadata: { ...stamp, bits_amount: bits },
      });
    }
    case 'link':
      return msg(say(pick(['clip', 'vod', 'this one']), link(pick(LINKS))));
    case 'gif':
      return msg(say(pick(['this stream is', 'chat rn', 'me watching that']), SAMPLE_GIF));
    case 'giant':
      return msg(say(pick(['HE DID IT', 'LETS GO', 'no way']), pick([GLITCH_CAT, COOL_CAT])), {
        tags: { ...extra.tags, 'msg-id': 'gigantified-emote-message' },
      });
  }
  return msg(flowLine(c));
}

/** A small starter batch so the flow preview isn't empty on the first frame. */
export function seedFlowMessages(n = 8): OverlayMessage[] {
  return Array.from({ length: n }, () => randomSampleMessage());
}
