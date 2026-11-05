// A member's sheet with every field set, written by hand. The fixture test
// proves the validator accepts it, and the sections test draws it. The golden
// file Rust writes (fixtures/profileSheet.member.json) is checked against the
// same validator.
import type { MemberProfileView, ProfileBadge, ProfileSheetView } from '../../types/profileView';

export const SAMPLE_BADGE: ProfileBadge = {
  id: 'sub-3',
  url: 'https://cdn.example/badges/sub-3.png',
  fallbacks: ['https://cdn.example/badges/sub-3-2x.png'],
  title: 'Subscriber',
  name: 'subscriber',
  description: 'Subscribed for 3 months',
  provider: 'twitch',
};

export const SAMPLE_MEMBER_PROFILE: MemberProfileView = {
  relics: [
    { id: 'relic-a', url: 'https://cdn.example/relics/a.png', title: 'Founding Relic', description: 'Early to the party' },
    { id: 'relic-b', url: 'https://cdn.example/relics/b.png', title: 'Second Relic' },
  ],
  hours: { label: 'Hours watched', value: '1,234', unit: 'hrs', roast: 'That is 51 days of other people playing games.' },
  twitch: {
    key: 'twitch',
    title: "Tyler's Twitch",
    tiles: [
      { key: 'twitch_age', label: 'Twitch age', value: '11 yrs, 7 mos', caption: 'Cake day in 135 days', detail: 'Joined March 20, 2014' },
      { key: 'followers', label: 'Followers', value: '12,345' },
      { key: 'account_type', label: 'Account type', value: 'Affiliate' },
    ],
  },
  lifetime: {
    key: 'lifetime',
    title: 'Lifetime in StreamNook',
    tiles: [
      { key: 'messages', label: 'Messages sent', value: '45,678', caption: 'into the void' },
      { key: 'streams', label: 'Streams watched', value: '321' },
      { key: 'cosmetics', label: 'Cosmetics', value: '12', caption: '3 paints, 4 badges, 5 StreamNook' },
      { key: 'member_rank', label: 'Member rank', value: '#7', caption: 'Mythic' },
    ],
  },
  views: '1,234',
  top_emotes: [
    { id: 'e1', name: 'Kappa', provider: 'twitch', url: 'https://cdn.example/e/kappa.png', count: 900, count_text: '900' },
    { id: 'e2', name: 'KEKW', provider: '7tv', url: 'https://cdn.example/e/kekw.png', count: 450, count_text: '450' },
  ],
  emote_totals: '2 emotes · 1,350 used',
  accolades: {
    earned: 2,
    total: 4,
    items: [
      { id: 'chatty', label: 'Chatty', icon: 'message-circle', colors: ['#60a5fa', '#2563eb'], earned: true, secret: false, hint: 'Messages sent: 45,678 / 1,000' },
      { id: 'night-owl', label: 'Night Owl', icon: 'moon', colors: ['#a78bfa', '#6d28d9'], earned: true, secret: false, hint: 'Collected' },
      { id: 'binge', label: 'Binge', icon: 'tv', colors: ['#34d399', '#047857'], earned: false, secret: false, hint: 'Streams watched: 321 / 500' },
      { id: 'secret-one', label: 'Secret', icon: 'ghost', colors: ['#94a3b8', '#475569'], earned: false, secret: true, hint: 'A hidden accolade.' },
    ],
  },
};

export const SAMPLE_SHEET: ProfileSheetView = {
  user_id: '1234',
  login: 'tyler',
  display_name: 'Tyler',
  shown_login: 'tyler',
  avatar_url: 'https://cdn.example/avatar.png',
  name_color: '#8B5CF6',
  groups: [{ title: 'Twitch', badges: [SAMPLE_BADGE] }],
  streamnook: { slug: 'streamnook-default', image_url: 'streamnook-house:streamnook-default', label: 'StreamNook' },
  member: { number: 7, tier: 'mythic', label: 'Mythic', accent: '#FBBF24', aura: 0.13 },
  member_profile: SAMPLE_MEMBER_PROFILE,
  page_link: {
    href: 'https://streamnook.app/u/tyler',
    url: 'https://streamnook.app/tyler',
    prefix: 'streamnook.app/',
    name: 'tyler',
    accent: '#FBBF24',
  },
  worn: [SAMPLE_BADGE],
  seventv_user_id: '01ABCDEF',
  theme: 'tier',
  frame: {
    url: 'https://cdn.example/frames/gold.png',
    slice: [199, 159, 199, 159],
    border: [18, 14],
  },
  accent: '#FBBF24',
  caption: 'Singularity',
};
