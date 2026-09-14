import { memo, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useActivityStore } from '../../stores/activityStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { useAppStore } from '../../stores/AppStore';
import { computePaintStyle, getBadgeImageUrl } from '../../services/seventvService';
import { activityHighlightStyle, colorForKind, labelForKind } from '../../utils/activityCategories';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import { ProviderLogo } from '../ProviderLogo';
import { parseKey } from '../../utils/providerKey';
import { CURRENCY_OPTIONS, symbolToCode, convert, formatMoney, preloadRates } from '../../services/currencyService';
import { Coins, SlidersHorizontal, Check } from 'lucide-react';
import { createPortal } from 'react-dom';
import { Tooltip } from '../ui/Tooltip';
import type { ActivityEvent, ActivityKind } from '../../types/activity';

// "combined" = all open sources (each row labeled with its stream); "current" =
// only the active tab. Blended mode forces combined.
type ActivityScope = 'combined' | 'current';

// Event kinds a viewer can toggle off per provider in the activity feed. Only the
// kinds each platform actually emits, so the filter list stays short.
const PROVIDER_EVENT_KINDS: Partial<Record<ProviderId, ActivityKind[]>> = {
  twitch: ['sub', 'resub', 'subgift', 'giftbomb', 'bits', 'raid', 'channelpoints', 'hypetrain', 'stream_online', 'stream_offline'],
  kick: ['sub', 'resub', 'subgift', 'giftbomb', 'follow', 'host'],
  youtube: ['superchat', 'supersticker', 'membership', 'member_milestone', 'subgift', 'giftbomb'],
  tiktok: ['gift', 'like', 'follow', 'share'],
};

const HIDDEN_STORAGE_KEY = 'sn-activity-hidden';
function loadHidden(): Set<string> {
  try {
    const raw = localStorage.getItem(HIDDEN_STORAGE_KEY);
    if (raw) return new Set(JSON.parse(raw) as string[]);
  } catch {
    /* ignore */
  }
  return new Set<string>();
}

// Borderless minimal pill (no bordered chips): muted when off, the accent/color
// when on. Reused for the scope toggle.
function FilterChip({
  active,
  color,
  onClick,
  children,
}: {
  active: boolean;
  color?: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`text-[10px] font-semibold uppercase tracking-wide transition-colors ${
        active ? '' : 'text-textMuted hover:text-textSecondary'
      }`}
      style={active ? { color: color ?? 'var(--color-accent)' } : undefined}
    >
      {children}
    </button>
  );
}

// Source (the broadcaster's) avatar shown when the feed is combined across
// streams. Only a handful of these (one per open source), unlike the per-chatter
// avatars we dropped, so the lookup stays cheap. Cached by source key AND persisted
// to localStorage: without it the broadcaster avatars vanished on reopen until each
// async re-resolve landed (and some providers' resolves can fail), so persisted
// saved events showed no source picture. The live resolve below still refreshes
// them and covers any new source.
const SOURCE_AVATAR_KEY = 'sn-source-avatars-v1';
function loadSourceAvatars(): Map<string, string> {
  try {
    const raw = localStorage.getItem(SOURCE_AVATAR_KEY);
    const obj = raw ? (JSON.parse(raw) as Record<string, string>) : {};
    return new Map(Object.entries(obj));
  } catch {
    return new Map();
  }
}
const sourceAvatarCache = loadSourceAvatars();
function persistSourceAvatars(): void {
  try {
    localStorage.setItem(SOURCE_AVATAR_KEY, JSON.stringify(Object.fromEntries(sourceAvatarCache)));
  } catch {
    /* quota exceeded or storage unavailable; the in-memory cache still works */
  }
}

function SourceAvatar({ provider, login, color }: { provider: ProviderId; login: string; color: string }) {
  const cacheKey = `${provider}:${login.toLowerCase()}`;
  const [url, setUrl] = useState<string | undefined>(() => sourceAvatarCache.get(cacheKey));
  useEffect(() => {
    if (url || !login) return;
    let active = true;
    // Per-provider avatar source: Twitch via Helix, Kick + YouTube from the channel
    // metadata the read adapters already cached at resolve time.
    const resolve = async (): Promise<string | undefined> => {
      if (provider === 'twitch') {
        const info = await invoke<{ profile_image_url?: string }>('get_user_by_login', { login }).catch(() => null);
        return info?.profile_image_url ?? undefined;
      }
      if (provider === 'kick') {
        const m = await invoke<{ profile_pic?: string | null } | null>('get_kick_channel_meta', { slug: login }).catch(() => null);
        return m?.profile_pic ?? undefined;
      }
      if (provider === 'youtube') {
        const m = await invoke<{ profile_pic?: string | null } | null>('get_youtube_channel_meta', { slug: login }).catch(() => null);
        return m?.profile_pic ?? undefined;
      }
      if (provider === 'tiktok') {
        const m = await invoke<{ profile_pic?: string | null } | null>('get_tiktok_channel_meta', { slug: login }).catch(() => null);
        return m?.profile_pic ?? undefined;
      }
      return undefined;
    };
    void resolve().then((u) => {
      if (!u) return;
      sourceAvatarCache.set(cacheKey, u);
      persistSourceAvatars();
      if (active) setUrl(u);
    });
    return () => {
      active = false;
    };
  }, [provider, login, url, cacheKey]);

  if (url) {
    return (
      <img
        src={url}
        alt=""
        loading="lazy"
        className="flex-shrink-0 rounded-full object-cover"
        style={{ height: '1.3em', width: '1.3em' }}
      />
    );
  }
  return (
    <span
      className="inline-block flex-shrink-0 rounded-full"
      style={{ height: '0.7em', width: '0.7em', backgroundColor: color }}
    />
  );
}

// Map a Twitch sub-plan tag to a compact tier label for the stat pill.
function subTierShort(tier?: string): string {
  switch (tier) {
    case '1000':
      return 'T1';
    case '2000':
      return 'T2';
    case '3000':
      return 'T3';
    case 'Prime':
      return 'Prime';
    default:
      return '';
  }
}

// The single headline stat for an event, shown as a colored pill (a la
// StreamElements' "16x T1"), replacing the old dot-separated text.
function statPill(e: ActivityEvent, targetCurrency: string): string | null {
  switch (e.kind) {
    case 'sub':
    case 'resub': {
      const t = subTierShort(e.tier);
      if (e.months && e.months > 1) return `${e.months}× ${t || 'sub'}`;
      return t || null;
    }
    case 'subgift':
      return subTierShort(e.tier) || null;
    case 'raid':
      return e.viewers != null ? `${e.viewers} raiders` : null;
    case 'bits':
      return e.amount != null ? `${e.amount} bits` : null;
    case 'superchat':
    case 'supersticker':
    case 'rant': {
      if (e.amount == null) return null;
      const original = `${e.currency ?? ''}${e.amount}`.trim();
      // Convert to the user's chosen currency when set + the rate is available;
      // otherwise show the original amount.
      if (!targetCurrency) return original;
      const from = symbolToCode(e.currency);
      if (!from) return original;
      const c = convert(e.amount, from, targetCurrency);
      return c != null ? formatMoney(c, targetCurrency) : original;
    }
    case 'like':
      return e.like_count != null ? `${e.like_count} likes` : null;
    default:
      return null;
  }
}

// Detail text for kinds that don't get a stat pill (channel points, hype train).
function detailFor(e: ActivityEvent): string {
  switch (e.kind) {
    case 'gift':
      return e.gift_name ? `${e.gift_name}${e.gift_count && e.gift_count > 1 ? ` x${e.gift_count}` : ''}` : '';
    case 'channelpoints':
    case 'hypetrain':
      return e.system_text ?? '';
    default:
      return '';
  }
}

type TpBadge = { id: string; imageUrl?: string; title?: string };
const EMPTY_TP_BADGES: TpBadge[] = [];

// The actor's badges + their painted/colored name, matching what chat shows
// (no avatar -- profile-pic fetches were the slow part). 7TV paint + 7TV/third-
// party badges are looked up live from the shared cosmetics store by the
// chatter's id (Twitch = bare id, others namespaced), populated when they chat;
// Twitch badges ride the event itself.
function ActorIdentity({ e }: { e: ActivityEvent }) {
  const cosmeticsKey = e.actor.id
    ? e.provider !== 'twitch'
      ? `${e.provider}:${e.actor.id}`
      : e.actor.id
    : undefined;
  const paint = useChatUserStore((s) => (cosmeticsKey ? s.users.get(cosmeticsKey)?.paint : undefined));
  const seventvBadge = useChatUserStore((s) => (cosmeticsKey ? s.users.get(cosmeticsKey)?.seventvBadge : undefined));
  const thirdPartyBadges = useChatUserStore((s) =>
    cosmeticsKey ? (s.users.get(cosmeticsKey)?.thirdPartyBadges as TpBadge[] | undefined) ?? EMPTY_TP_BADGES : EMPTY_TP_BADGES,
  );
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';

  const name = e.actor.display_name || e.actor.username || 'Someone';
  const nameStyle = useMemo<CSSProperties>(
    () =>
      paint
        ? (computePaintStyle(paint, e.actor.color, paintShadowMode) as CSSProperties)
        : { color: e.actor.color || undefined },
    [paint, e.actor.color, paintShadowMode],
  );
  const twitchBadges = e.actor.badges ?? [];
  const hasBadges = twitchBadges.length > 0 || !!seventvBadge || thirdPartyBadges.length > 0;

  return (
    <span className="flex min-w-0 items-center gap-1">
      {e.actor.avatar_url && (
        <img
          src={e.actor.avatar_url}
          alt=""
          className="inline-block flex-shrink-0 rounded-full object-cover"
          style={{ width: '1.35em', height: '1.35em' }}
          onError={(ev) => {
            ev.currentTarget.style.display = 'none';
          }}
        />
      )}
      {hasBadges && (
        <span className="inline-flex flex-shrink-0 items-center gap-0.5">
          {twitchBadges.map((b, i) => {
            const url = b.info?.image_url_2x || b.info?.image_url_1x;
            return url ? (
              <Tooltip key={`tb-${b.key}-${i}`} content={b.info?.title ?? ''}>
                <img
                  src={url}
                  alt={b.info?.title ?? ''}
                  className="inline-block"
                  style={{ height: '1.15em' }}
                />
              </Tooltip>
            ) : null;
          })}
          {seventvBadge && (
            <img src={getBadgeImageUrl(seventvBadge)} alt="" className="inline-block" style={{ height: '1.15em' }} />
          )}
          {thirdPartyBadges
            .filter((b) => b?.imageUrl)
            .map((b, i) => (
              <Tooltip key={`tp-${b.id}-${i}`} content={b.title ?? ''}>
                <img
                  src={b.imageUrl}
                  alt={b.title ?? ''}
                  className="inline-block"
                  style={{ height: '1.15em' }}
                />
              </Tooltip>
            ))}
        </span>
      )}
      <span className="truncate font-bold" style={nameStyle}>
        {name}
      </span>
    </span>
  );
}

interface ActivityFeedWidgetProps {
  // Composite source keys ("provider:channel") currently open in this window.
  activeKeys: string[];
  // The active tab's composite key (for the "current stream" scope).
  currentKey?: string | null;
  // Blended mode (one streamer across platforms) forces the combined scope.
  blended?: boolean;
  // Composite key + bare login -> display name, for the per-row source label.
  sources?: Record<string, string>;
}

const TOP_STICK_PX = 60;

const ActivityRow = memo(function ActivityRow({
  e,
  fontSize,
  multiSource,
  sourceName,
  targetCurrency,
}: {
  e: ActivityEvent;
  fontSize: number;
  multiSource: boolean;
  sourceName?: string;
  targetCurrency: string;
}) {
  const provider = PROVIDERS[e.provider];
  // Color the row by PROVIDER so platforms are visually separated (Twitch purple,
  // Kick green, YouTube red), instead of by kind where a Twitch sub and a Kick sub
  // were both purple. The kind is still conveyed by the label + icon + stat pill.
  const color = provider?.color ?? colorForKind(e.kind);
  const time = useMemo(() => {
    try {
      return new Date(e.timestamp).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    } catch {
      return '';
    }
  }, [e.timestamp]);

  // When the feed spans several streams, label each row with its source (the
  // broadcaster's avatar + name, provider-colored so the same streamer's Twitch
  // vs Kick vs YouTube rows are distinguishable in blended mode). Otherwise just
  // tag the platform.
  const sourceLogin = parseKey(e.channel).channel;
  const meta = (
    <span className="ml-auto flex min-w-0 flex-shrink-0 items-center gap-1">
      {multiSource ? (
        <>
          {/* Platform mark leads, then the broadcaster's avatar + name, so the
              source's provider (Twitch/Kick/YouTube) reads first. */}
          <ProviderLogo provider={e.provider} size={Math.max(10, Math.round(fontSize * 0.78))} />
          <SourceAvatar provider={e.provider} login={sourceLogin} color={provider?.color ?? '#888'} />
          <Tooltip content={sourceName || sourceLogin}>
            <span
              className="max-w-[8em] truncate font-semibold"
              style={{ fontSize: '0.62em', color: provider?.color }}
            >
              {sourceName || sourceLogin}
            </span>
          </Tooltip>
        </>
      ) : (
        <span className="font-semibold uppercase tracking-wide" style={{ fontSize: '0.62em', color: provider?.color }}>
          {provider?.label ?? e.provider}
        </span>
      )}
      <span className="flex-shrink-0 text-textMuted" style={{ fontSize: '0.62em' }}>
        {time}
      </span>
    </span>
  );

  // Community gift bomb: one collapsed row for the whole batch, sized up and
  // animated (spring pop + a one-time sheen sweep) so a big drop stands out.
  if (e.kind === 'giftbomb') {
    const count = e.gift_count ?? 0;
    const tier = subTierShort(e.tier);
    return (
      <motion.div
        layout
        initial={{ opacity: 0, scale: 0.92 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0 }}
        transition={{
          layout: { type: 'spring', stiffness: 500, damping: 40 },
          scale: { type: 'spring', stiffness: 420, damping: 22 },
          opacity: { duration: 0.2 },
        }}
        className="sn-giftbomb-border relative flex items-center gap-2 rounded-lg px-3"
        style={{
          fontSize: `${fontSize}px`,
          minHeight: '3.9em',
          // Glassy color-tinted fill; the animated ring (.sn-giftbomb-border) is the
          // real standout. A plain flex row, so the meta's ml-auto sits right again.
          background: `linear-gradient(100deg, ${color}4d, ${color}1a 60%, ${color}08)`,
          ['--sn-bomb-color' as string]: color,
        }}
      >
        <ActorIdentity e={e} />
        <svg
          className="flex-shrink-0"
          style={{ width: '1.25em', height: '1.25em', color }}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="20 12 20 22 4 22 4 12" />
          <rect x="2" y="7" width="20" height="5" />
          <line x1="12" y1="22" x2="12" y2="7" />
          <path d="M12 7H7.5a2.5 2.5 0 0 1 0-5C11 2 12 7 12 7z" />
          <path d="M12 7h4.5a2.5 2.5 0 0 0 0-5C13 2 12 7 12 7z" />
        </svg>
        <span className="flex-shrink-0 text-textMuted" style={{ fontSize: '0.85em' }}>
          gifted
        </span>
        <span
          className="flex-shrink-0 rounded font-black tabular-nums"
          style={{ fontSize: '1.2em', padding: '0.05em 0.55em', backgroundColor: color, color: '#fff' }}
        >
          {count}
        </span>
        <span className="flex-shrink-0 text-textMuted" style={{ fontSize: '0.85em' }}>
          {e.provider === 'youtube' ? 'memberships' : `subs${tier ? ` · ${tier}` : ''}`}
        </span>
        {meta}
      </motion.div>
    );
  }

  // Paid gift (TikTok roses etc.): highlight it like a gifted sub / membership gift.
  // The animated ring + tinted fill + the gift's own (often animated) icon make a
  // paid event stand out from the free likes below it.
  if (e.kind === 'gift') {
    const count = e.gift_count ?? 1;
    return (
      <motion.div
        layout
        initial={{ opacity: 0, scale: 0.92 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0 }}
        transition={{
          layout: { type: 'spring', stiffness: 500, damping: 40 },
          scale: { type: 'spring', stiffness: 420, damping: 22 },
          opacity: { duration: 0.2 },
        }}
        className="sn-giftbomb-border relative flex items-center gap-2 rounded-lg px-3"
        style={{
          fontSize: `${fontSize}px`,
          minHeight: '3em',
          background: `linear-gradient(100deg, ${color}4d, ${color}1a 60%, ${color}08)`,
          ['--sn-bomb-color' as string]: color,
        }}
      >
        <ActorIdentity e={e} />
        <span className="flex-shrink-0 text-textMuted" style={{ fontSize: '0.85em' }}>
          sent
        </span>
        {e.gift_image_url && (
          <Tooltip content={e.gift_name ?? ''}>
            <img
              src={e.gift_image_url}
              alt={e.gift_name ?? ''}
              className="flex-shrink-0"
              style={{ width: '1.9em', height: '1.9em' }}
              onError={(ev) => {
                ev.currentTarget.style.display = 'none';
              }}
            />
          </Tooltip>
        )}
        {e.gift_name && (
          <span className="min-w-0 flex-shrink truncate font-bold" style={{ fontSize: '0.95em' }}>
            {e.gift_name}
          </span>
        )}
        {count > 1 && (
          <span
            className="flex-shrink-0 rounded font-black tabular-nums"
            style={{ fontSize: '1.1em', padding: '0.05em 0.5em', backgroundColor: color, color: '#fff' }}
          >
            {'×'}{count}
          </span>
        )}
        {e.amount != null && e.amount > 0 && (
          <span
            className="flex-shrink-0 rounded font-bold tabular-nums"
            style={{ fontSize: '0.8em', padding: '0.05em 0.45em', backgroundColor: `${color}33`, color: '#fff' }}
          >
            {e.amount} {'\u{1f48e}'}
          </span>
        )}
        {meta}
      </motion.div>
    );
  }

  // Prime subs are their own occasion: show the Prime mark + "via Prime" instead
  // of a bland "Nx Prime" pill, with the month count as a quiet trailing detail.
  const isPrime = (e.kind === 'sub' || e.kind === 'resub') && e.tier === 'Prime';
  const pill = isPrime ? null : statPill(e, targetCurrency);
  const detail = pill || isPrime ? '' : detailFor(e);

  return (
    <motion.div
      layout
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ layout: { type: 'spring', stiffness: 500, damping: 40 }, opacity: { duration: 0.2 } }}
      className="rounded-md border px-2.5 py-1.5"
      style={{ ...activityHighlightStyle(color), fontSize: `${fontSize}px` }}
    >
      <div className="flex items-center gap-1.5">
        <ActorIdentity e={e} />
        <span className="flex-shrink-0 text-textMuted" style={{ fontSize: '0.85em' }}>
          {labelForKind(e.kind)}
        </span>
        {isPrime ? (
          <>
            <span
              className="flex flex-shrink-0 items-center gap-1 font-bold"
              style={{ fontSize: '0.9em', color: '#60a5fa' }}
            >
              <svg style={{ width: '1.15em', height: '1.15em' }} fill="currentColor" viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  clipRule="evenodd"
                  d="M18 5v8a2 2 0 0 1-2 2H4a2.002 2.002 0 0 1-2-2V5l4 3 4-4 4 4 4-3z"
                />
              </svg>
              via Prime
            </span>
            {/* Months matter to streamers ("thank you for N months") - so for
                Prime, show them as their own highlighted pill, not a quiet aside. */}
            {e.months != null && e.months > 1 && (
              <span
                className="flex-shrink-0 rounded font-bold tabular-nums"
                style={{ fontSize: '1em', padding: '0.1em 0.5em', backgroundColor: `${color}33`, color: '#fff' }}
              >
                {e.months} mo
              </span>
            )}
          </>
        ) : pill ? (
          <span
            className="flex-shrink-0 rounded font-bold tabular-nums"
            style={{ fontSize: '1em', padding: '0.1em 0.5em', backgroundColor: `${color}33`, color: '#fff' }}
          >
            {pill}
          </span>
        ) : null}
        {e.streak != null && (
          <span className="flex-shrink-0 text-textMuted" style={{ fontSize: '0.72em' }}>
            {e.streak} streak
          </span>
        )}
        {detail && (
          <span className="truncate text-textSecondary" style={{ fontSize: '0.85em' }}>
            {detail}
          </span>
        )}
        {meta}
      </div>
      {e.message && (
        <div className="mt-0.5 truncate pl-1 text-textMuted" style={{ fontSize: '0.85em' }}>
          {e.message}
        </div>
      )}
    </motion.div>
  );
});

export const ActivityFeedWidget = memo(function ActivityFeedWidget({
  activeKeys,
  currentKey,
  blended,
  sources,
}: ActivityFeedWidgetProps) {
  const events = useActivityStore((s) => s.events);
  const fontSize = useAppStore((s) => s.settings.chat_design?.activity_font_size) ?? 14;

  // Scope: combined (all open sources, each row source-labeled) vs the current
  // tab only. Blended mode (one streamer across platforms) is always combined.
  const [scope, setScope] = useState<ActivityScope>('combined');
  // Per-provider event filter: a persisted set of "<provider>:<kind>" the viewer
  // has hidden from the feed.
  const [hidden, setHidden] = useState<Set<string>>(loadHidden);
  const [filterOpen, setFilterOpen] = useState(false);
  const [filterPos, setFilterPos] = useState<{ right: number; bottom: number } | null>(null);
  const filterBtnRef = useRef<HTMLButtonElement>(null);
  const [currencyOpen, setCurrencyOpen] = useState(false);
  const [currencyPos, setCurrencyPos] = useState<{ right: number; bottom: number } | null>(null);
  const currencyBtnRef = useRef<HTMLButtonElement>(null);
  const toggleHidden = (provider: ProviderId, kind: ActivityKind) => {
    setHidden((prev) => {
      const next = new Set(prev);
      const k = `${provider}:${kind}`;
      if (next.has(k)) next.delete(k);
      else next.add(k);
      try {
        localStorage.setItem(HIDDEN_STORAGE_KEY, JSON.stringify([...next]));
      } catch {
        /* ignore */
      }
      return next;
    });
  };
  const openProviders = useMemo(() => {
    const seen = new Set<ProviderId>();
    for (const k of activeKeys) {
      const p = parseKey(k).provider as ProviderId;
      if (PROVIDER_EVENT_KINDS[p]) seen.add(p);
    }
    return [...seen];
  }, [activeKeys]);

  // Reload the per-provider filter when a Go Live profile is applied (it writes the
  // hidden set to localStorage from the title bar, another component).
  useEffect(() => {
    const reload = () => setHidden(loadHidden());
    window.addEventListener('sn-activity-hidden-changed', reload);
    return () => window.removeEventListener('sn-activity-hidden-changed', reload);
  }, []);
  // Optional target currency for Super Chats (persisted): convert every super chat
  // to this currency. '' = off (show the original). Rates load lazily on selection.
  const [targetCurrency, setTargetCurrency] = useState(() => localStorage.getItem('sn_superchat_currency') || '');
  useEffect(() => {
    if (targetCurrency) preloadRates();
  }, [targetCurrency]);
  const effectiveScope: ActivityScope = blended ? 'combined' : scope;
  const shownKeys = useMemo(() => {
    if (effectiveScope === 'current' && currentKey) return [currentKey.toLowerCase()];
    return activeKeys.map((k) => k.toLowerCase());
  }, [effectiveScope, currentKey, activeKeys]);
  const shownSet = useMemo(() => new Set(shownKeys), [shownKeys]);
  const multiSource = shownKeys.length > 1;
  // The toggle only matters when it can change something (not blended, 2+ open).
  const showScopeToggle = !blended && activeKeys.length > 1;

  // Two-click purge of the SHOWN sources' history. Clears the in-memory feed AND
  // the persisted store; sources not currently shown keep their saved history.
  const [confirmPurge, setConfirmPurge] = useState(false);
  const purgeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (purgeTimer.current) clearTimeout(purgeTimer.current); }, []);
  const handlePurge = () => {
    if (confirmPurge) {
      if (purgeTimer.current) clearTimeout(purgeTimer.current);
      setConfirmPurge(false);
      useActivityStore.getState().purgeChannels(shownKeys);
    } else {
      setConfirmPurge(true);
      purgeTimer.current = setTimeout(() => setConfirmPurge(false), 3000);
    }
  };

  const visible = useMemo(
    () =>
      events.filter(
        (e) => shownSet.has(e.channel.toLowerCase()) && !hidden.has(`${e.provider}:${e.kind}`),
      ),
    [events, shownSet, hidden],
  );

  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (el && el.scrollTop <= TOP_STICK_PX) el.scrollTop = 0;
  }, [visible.length]);

  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex items-center gap-3 px-3 py-1.5 text-xs font-semibold uppercase tracking-wide text-textMuted">
        <span>Activity</span>
        {showScopeToggle && (
          <div className="flex items-center gap-2">
            <FilterChip active={scope === 'combined'} onClick={() => setScope('combined')}>
              All
            </FilterChip>
            <FilterChip active={scope === 'current'} onClick={() => setScope('current')}>
              This stream
            </FilterChip>
          </div>
        )}
        <div className="ml-auto flex items-center gap-2">
          {/* Super Chat currency converter. The coin icon + label make it read as a
              currency control (not bare text); pick a target to convert every super
              chat, "Orig" leaves amounts as sent. Tints to the accent when a
              conversion is active. */}
          {/* Themed currency picker (replaces the native select, whose popup
              WebView2 renders unstyled). Shows the active code; opens a glass menu. */}
          <Tooltip content="Convert YouTube Super Chats to this currency">
            <button
              ref={currencyBtnRef}
              type="button"
              onClick={() => {
                const r = currencyBtnRef.current?.getBoundingClientRect();
                if (r) {
                  setCurrencyPos({ right: window.innerWidth - r.right, bottom: window.innerHeight - r.top + 6 });
                }
                setCurrencyOpen((v) => !v);
              }}
              className={`flex items-center gap-1 transition-colors ${
                targetCurrency || currencyOpen ? 'text-accent' : 'text-textMuted hover:text-textSecondary'
              }`}
            >
              <Coins size={12} className="flex-shrink-0" />
              <span className="text-[10px] font-semibold normal-case tracking-normal">Super Chat</span>
              <span className="text-[10px] font-semibold uppercase tracking-wide">{targetCurrency || 'Orig'}</span>
              <svg
                className={`h-2.5 w-2.5 transition-transform ${currencyOpen ? 'rotate-180' : ''}`}
                viewBox="0 0 20 20"
                fill="currentColor"
              >
                <path
                  fillRule="evenodd"
                  d="M5.3 7.3a1 1 0 011.4 0L10 10.6l3.3-3.3a1 1 0 111.4 1.4l-4 4a1 1 0 01-1.4 0l-4-4a1 1 0 010-1.4z"
                  clipRule="evenodd"
                />
              </svg>
            </button>
          </Tooltip>
          {openProviders.length > 0 && (
            <Tooltip content="Choose which events show in the activity feed">
              <button
                ref={filterBtnRef}
                type="button"
                onClick={() => {
                  const r = filterBtnRef.current?.getBoundingClientRect();
                  if (r) {
                    setFilterPos({ right: window.innerWidth - r.right, bottom: window.innerHeight - r.top + 6 });
                  }
                  setFilterOpen((v) => !v);
                }}
                className={`flex flex-shrink-0 items-center transition-colors ${
                  filterOpen || hidden.size > 0 ? 'text-accent' : 'text-textMuted hover:text-textSecondary'
                }`}
              >
                <SlidersHorizontal size={13} />
              </button>
            </Tooltip>
          )}
          <span>{visible.length}</span>
          {visible.length > 0 && (
            <Tooltip content={confirmPurge ? 'Click again to clear this view' : 'Clear shown activity history'}>
              <button
                type="button"
                onClick={handlePurge}
                className="flex-shrink-0 transition-colors hover:text-red-400"
              >
                {confirmPurge ? (
                  <span className="text-[10px] font-semibold normal-case text-red-400">Clear?</span>
                ) : (
                  <svg
                    className="h-3.5 w-3.5"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <polyline points="3 6 5 6 21 6" />
                    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                  </svg>
                )}
              </button>
            </Tooltip>
          )}
        </div>
      </div>
      {currencyOpen && currencyPos && createPortal(
        <>
          <div className="fixed inset-0 z-[1000]" onClick={() => setCurrencyOpen(false)} />
          <div
            className="glass-panel fixed z-[1001] max-h-[60vh] w-28 overflow-y-auto rounded-lg border border-borderLight p-1 shadow-xl scrollbar-thin"
            // Opaque themed surface: a live backdrop-blur flickers over the feed.
            style={{ right: currencyPos.right, bottom: currencyPos.bottom, backgroundColor: 'var(--color-background-tertiary)', backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
          >
            {['', ...CURRENCY_OPTIONS].map((c) => {
              const selected = targetCurrency === c;
              return (
                <button
                  key={c || 'orig'}
                  type="button"
                  onClick={() => {
                    setTargetCurrency(c);
                    try {
                      localStorage.setItem('sn_superchat_currency', c);
                    } catch {
                      /* ignore */
                    }
                    setCurrencyOpen(false);
                  }}
                  className={`flex w-full items-center justify-between gap-2 rounded-md px-2.5 py-1.5 text-left text-xs transition-colors ${
                    selected ? 'bg-accent/20 text-accent' : 'text-textSecondary hover:bg-white/5 hover:text-textPrimary'
                  }`}
                >
                  <span className="font-semibold uppercase tracking-wide">{c || 'Orig'}</span>
                  {selected && <Check size={13} className="flex-shrink-0 text-accent" />}
                </button>
              );
            })}
          </div>
        </>,
        document.body,
      )}
      {filterOpen && filterPos && createPortal(
        <>
          <div className="fixed inset-0 z-[1000]" onClick={() => setFilterOpen(false)} />
          <div
            className="glass-panel fixed z-[1001] max-h-[60vh] w-56 overflow-y-auto rounded-lg border border-borderLight p-2 shadow-xl scrollbar-thin"
            // Opaque themed surface: a live backdrop-blur flickers over the feed.
            style={{ right: filterPos.right, bottom: filterPos.bottom, backgroundColor: 'var(--color-background-tertiary)', backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
          >
            <div className="mb-1 px-1 text-[10px] font-semibold uppercase tracking-wide text-textMuted">
              Show in activity
            </div>
            {openProviders.map((p) => (
              <div key={p} className="mb-1.5">
                <div className="mb-0.5 flex items-center gap-1.5 px-1">
                  <ProviderLogo provider={p} size={12} />
                  <span className="text-[11px] font-semibold" style={{ color: PROVIDERS[p].color }}>
                    {PROVIDERS[p].label}
                  </span>
                </div>
                {(PROVIDER_EVENT_KINDS[p] ?? []).map((kind) => {
                  const shown = !hidden.has(`${p}:${kind}`);
                  return (
                    <button
                      key={kind}
                      type="button"
                      onClick={() => toggleHidden(p, kind)}
                      className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-xs hover:bg-white/5"
                    >
                      <span
                        className={`flex h-3.5 w-3.5 flex-shrink-0 items-center justify-center rounded-sm border ${
                          shown ? 'border-accent bg-accent' : 'border-borderLight'
                        }`}
                      >
                        {shown && (
                          <svg viewBox="0 0 12 12" className="h-2.5 w-2.5" fill="none" stroke="#000" strokeWidth="2.5">
                            <path d="M2 6.5l2.5 2.5L10 3" />
                          </svg>
                        )}
                      </span>
                      <span className={`capitalize ${shown ? 'text-textSecondary' : 'text-textMuted line-through'}`}>
                        {labelForKind(kind)}
                      </span>
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </>,
        document.body,
      )}
      <div ref={scrollRef} className="scrollbar-thin min-h-0 flex-1 space-y-1 overflow-y-auto px-2 pb-2">
        {visible.length === 0 ? (
          <div className="px-2 py-6 text-center text-xs text-textMuted">
            Subs, raids, gifts and other events from your sources will appear here.
          </div>
        ) : (
          <AnimatePresence initial={false}>
            {visible.map((e) => (
              <ActivityRow
                key={e.id}
                e={e}
                fontSize={fontSize}
                multiSource={multiSource}
                sourceName={sources?.[e.channel.toLowerCase()] ?? sources?.[e.channel]}
                targetCurrency={targetCurrency}
              />
            ))}
          </AnimatePresence>
        )}
      </div>
    </div>
  );
});
