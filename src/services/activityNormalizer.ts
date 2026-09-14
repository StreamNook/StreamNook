import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useActivityStore } from '../stores/activityStore';
import type { ActivityEvent, ActivityKind } from '../types/activity';
import type { ProviderId } from '../types/providers';
import { makeKey } from '../utils/providerKey';
import { Logger } from '../utils/logger';

// Read-only bridge: observes every provider's event streams and mirrors them
// into the MultiChat activity store as normalized ActivityEvents. It never
// writes chat or alters any existing behavior; it only listens. Sources:
//   1. `twitch-subscription-detected` - the chat store dispatches it for Twitch
//      USERNOTICE frames (subs, resubs, gifts, raids); carries the channel.
//   2. `provider-activity-detected` - the chat store dispatches it for non-Twitch
//      event messages (Kick subs/gifts today, follows/raids/hosts once wired),
//      already carrying the composite source key.
//   3. EventSub `online`/`offline` Tauri emits - carry the broadcaster login.
// Channel-points and hype-train emits don't carry a broadcaster login, so they
// are left for a later pass that resolves the logged-in user's channel.
//
// Only runs while a MultiChat window with the activity feed enabled is mounted.

let started = false;
const cleanups: Array<UnlistenFn | (() => void)> = [];
let seq = 0;

interface SubDetectedDetail {
  login?: string;
  msgId?: string;
  displayName?: string;
  rawMessage?: string;
}

const MSGID_TO_KIND: Record<string, ActivityKind> = {
  sub: 'sub',
  resub: 'resub',
  subgift: 'subgift',
  submysterygift: 'giftbomb',
  giftpaidupgrade: 'sub',
  primepaidupgrade: 'sub',
  anongiftpaidupgrade: 'sub',
  raid: 'raid',
  channelpoints: 'channelpoints',
  // YouTube events (stamped by the youtube adapter): Super Chats / stickers and
  // memberships. The activity kinds + categories already exist for these.
  superchat: 'superchat',
  supersticker: 'supersticker',
  membership: 'membership',
  // A tenure milestone ("Member for 6 months"), split from a brand-new member by
  // the chat store using the milestone tag the YouTube adapter stamps.
  member_milestone: 'member_milestone',
  membergift: 'giftbomb',
  // TikTok events (stamped by the tiktok adapter): gifts/roses, follows, shares,
  // hearts. The activity kinds + categories already exist for these.
  tiktok_gift: 'gift',
  tiktok_follow: 'follow',
  tiktok_share: 'share',
  tiktok_like: 'like',
};

function push(partial: Omit<ActivityEvent, 'id' | 'timestamp'> & { id?: string; timestamp?: string }): void {
  const ev = {
    id: partial.id ?? `act-${Date.now()}-${seq++}`,
    timestamp: partial.timestamp ?? new Date().toISOString(),
    ...partial,
  } as ActivityEvent;
  useActivityStore.getState().addEvent(ev);
}

// Pull the lowercased channel login out of a raw USERNOTICE IRC frame.
function channelFromRaw(raw: string | undefined): string | null {
  const m = raw?.match(/USERNOTICE\s+#(\S+)/i);
  return m ? m[1].toLowerCase() : null;
}

function onSubscriptionDetected(ev: Event): void {
  const detail = (ev as CustomEvent<SubDetectedDetail>).detail || {};
  const kind = MSGID_TO_KIND[(detail.msgId || '').toLowerCase()];
  if (!kind) return;
  // TEMP [activity-dedup]: this RAW path has no gift-bomb dedup. If it fires for
  // subgifts (it shouldn't normally — the structured path handles Twitch subs), it
  // would produce the un-collapsed "5 subs + total" rows. Strip after diagnosing.
  if (kind === 'giftbomb' || kind === 'subgift') {
    Logger.info(`[activity-dedup] RAW path ${kind} login=${detail.login ?? '?'}`);
  }
  const channel = channelFromRaw(detail.rawMessage);
  if (!channel) return;
  push({
    provider: 'twitch',
    channel: makeKey('twitch', channel),
    kind,
    actor: { username: detail.login || '', display_name: detail.displayName || detail.login || '' },
  });
}

interface ProviderActivityDetail {
  provider?: ProviderId;
  channelKey?: string; // already the composite "<provider>:<channel>" source key
  msgId?: string;
  username?: string;
  displayName?: string;
  userId?: string;
  color?: string;
  months?: number;
  streak?: number;
  tier?: string;
  giftCount?: number;
  giftName?: string;
  giftImage?: string;
  giftDiamonds?: number;
  likeCount?: number;
  avatarUrl?: string;
  originId?: string;
  badges?: ActivityEvent['actor']['badges'];
  systemText?: string;
  amount?: number; // super chat amount
  currency?: string; // super chat currency symbol
  message?: string; // super chat / event comment
}

// Community gift bombs arrive as a `submysterygift` ("X gifted N subs") followed
// by N individual `subgift` events sharing the same origin id. Remember each bomb
// briefly so its follow-up subgifts collapse into the one bomb row instead of
// adding N more. Keyed by origin id -> expiry ms (lazily pruned).
const giftBombOrigins = new Map<string, number>();
const GIFT_BOMB_WINDOW_MS = 60_000;

// Non-Twitch events (Kick subs/gifts today; follows/raids/hosts once wired) the
// chat store mirrors out as a generic event, already carrying the composite key.
function onProviderActivity(ev: Event): void {
  const d = (ev as CustomEvent<ProviderActivityDetail>).detail || {};
  const kind = MSGID_TO_KIND[(d.msgId || '').toLowerCase()];
  if (!kind || !d.channelKey || !d.provider) return;

  // TEMP [activity-dedup]: trace the structured path's gift-bomb collapse — shows the
  // originId and whether the bomb was already recorded (so subgifts get dropped).
  if (kind === 'giftbomb' || kind === 'subgift') {
    Logger.info(
      `[activity-dedup] structured ${kind} origin=${d.originId ?? 'none'} bombKnown=${d.originId ? giftBombOrigins.has(d.originId) : false}`,
    );
  }

  const now = Date.now();
  if (d.originId) {
    // Prune expired bomb windows so the map can't grow without bound.
    for (const [id, exp] of giftBombOrigins) if (exp < now) giftBombOrigins.delete(id);
    if (kind === 'giftbomb') {
      giftBombOrigins.set(d.originId, now + GIFT_BOMB_WINDOW_MS);
    } else if (kind === 'subgift' && giftBombOrigins.has(d.originId)) {
      // This individual gift belongs to a bomb already shown as one row. Drop it.
      return;
    }
  }

  push({
    provider: d.provider,
    channel: d.channelKey,
    kind,
    actor: {
      id: d.userId,
      username: d.username || '',
      display_name: d.displayName || d.username || '',
      color: d.color,
      avatar_url: d.avatarUrl,
      badges: d.badges,
    },
    months: d.months,
    streak: d.streak,
    tier: d.tier,
    gift_count: d.giftCount,
    gift_name: d.giftName,
    gift_image_url: d.giftImage,
    like_count: d.likeCount,
    system_text: d.systemText,
    amount: d.amount ?? d.giftDiamonds,
    currency: d.currency,
    message: d.message,
  });
}

export async function startActivityNormalizer(): Promise<void> {
  if (started) return;
  started = true;

  window.addEventListener('twitch-subscription-detected', onSubscriptionDetected as EventListener);
  cleanups.push(() => window.removeEventListener('twitch-subscription-detected', onSubscriptionDetected as EventListener));

  window.addEventListener('provider-activity-detected', onProviderActivity as EventListener);
  cleanups.push(() => window.removeEventListener('provider-activity-detected', onProviderActivity as EventListener));

  const onOnline = await listen<{ broadcaster_user_login?: string; broadcaster_user_name?: string }>(
    'eventsub://online',
    (e) => {
      const login = (e.payload?.broadcaster_user_login || '').toLowerCase();
      if (!login) return;
      push({
        provider: 'twitch',
        channel: makeKey('twitch', login),
        channel_display: e.payload?.broadcaster_user_name,
        kind: 'stream_online',
        actor: { username: login, display_name: e.payload?.broadcaster_user_name || login },
      });
    },
  );
  cleanups.push(onOnline);

  const onOffline = await listen<{ broadcaster_user_login?: string; broadcaster_user_name?: string }>(
    'eventsub://offline',
    (e) => {
      const login = (e.payload?.broadcaster_user_login || '').toLowerCase();
      if (!login) return;
      push({
        provider: 'twitch',
        channel: makeKey('twitch', login),
        channel_display: e.payload?.broadcaster_user_name,
        kind: 'stream_offline',
        actor: { username: login, display_name: e.payload?.broadcaster_user_name || login },
      });
    },
  );
  cleanups.push(onOffline);
}

export function stopActivityNormalizer(): void {
  if (!started) return;
  started = false;
  for (const c of cleanups.splice(0)) {
    try {
      c();
    } catch {
      /* best-effort teardown */
    }
  }
}
