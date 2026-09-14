// Rewards: drops and badges, the two things you earn by watching.
//
// Named Activity until it was pointed out that the word describes neither of
// the things on it. Both tabs are collections of rewards, so the tab says that.
//
// Uses the same backend surface as the desktop Drops Center; connecting uses
// the device-code flow directly (the desktop's authorize popup is
// desktop-gated), showing the code here and opening the browser, mirroring the
// main Twitch login pattern on mobile.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { ArrowSquareOut, CalendarBlank, CaretRight, CheckCircle, Gift, MagnifyingGlass, Warning, X } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { PullToRefresh } from '../ui/PullToRefresh';
import { MobileSheet } from '../ui/MobileSheet';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { getAllUserBadgesWithEarned } from '../../services/badgeService';
import {
  deriveBadgeStatus,
  formatBadgeDateInfo,
  type BadgeWindowStatus,
} from '../../utils/badgeWindow';
import { gameBoxArt } from '../../utils/boxArt';
import { openExternal } from '../openExternal';
import { Logger } from '../../utils/logger';
import type {
  DropCampaign,
  DropsDeviceCodeInfo,
  InventoryItem,
  InventoryResponse,
} from '../../types';

type RewardsTab = 'drops' | 'badges';

interface GameGroup<T> {
  game: string;
  art: string;
  items: T[];
}

function groupByGame<T>(
  items: T[],
  getGame: (t: T) => string,
  getArt: (t: T) => string,
): GameGroup<T>[] {
  const map = new Map<string, GameGroup<T>>();
  for (const item of items) {
    const game = getGame(item) || 'Other';
    const existing = map.get(game);
    if (existing) existing.items.push(item);
    else map.set(game, { game, art: getArt(item), items: [item] });
  }
  // Busiest game first, then alphabetical. The tiebreak matters: without it the
  // order follows whatever the API happened to return and the list reshuffles
  // itself under the reader on every poll.
  return [...map.values()].sort(
    (a, b) => b.items.length - a.items.length || a.game.localeCompare(b.game),
  );
}

/**
 * One game, collapsed to a single row until you open it.
 *
 * A game routinely runs half a dozen campaigns at once, and listing them all
 * flat turned this screen into the same box art repeated down the page. Closed
 * by default means the list answers "which games have drops" at a glance, and
 * opening one is a deliberate ask for the detail.
 *
 * Searching overrides it and opens everything that matched, because hiding
 * results behind another tap defeats the point of having searched.
 */
const GameGroupSection: React.FC<{
  art: string;
  game: string;
  count: number;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}> = ({ art, game, count, expanded, onToggle, children }) => (
  <div>
    <button
      onClick={onToggle}
      aria-expanded={expanded}
      className="w-full flex items-center gap-2 mt-3 mb-1.5 text-left active:opacity-70 transition-opacity"
    >
      {/* Through gameBoxArt, not straight into src. Twitch hands back box art in
          two shapes and one of them is a TEMPLATE carrying literal
          {width}x{height} placeholders, which is not a loadable URL. The
          Available now campaigns come back in that shape, which is why they
          were the ones rendering with no art at all. */}
      {art ? (
        <img
          src={gameBoxArt(art, 144, 192)}
          alt=""
          loading="lazy"
          draggable={false}
          className="w-10 aspect-[3/4] object-cover rounded shrink-0"
        />
      ) : (
        <div className="w-10 aspect-[3/4] rounded bg-surface shrink-0" />
      )}
      <div className="flex-1 min-w-0">
        <div className="text-[14px] font-semibold text-textPrimary truncate">{game}</div>
        <div className="text-[12px] text-textMuted">
          {count} {count === 1 ? 'campaign' : 'campaigns'}
        </div>
      </div>
      <motion.span
        className="ml-auto shrink-0 flex text-textMuted"
        animate={{ rotate: expanded ? 90 : 0 }}
        transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
      >
        <CaretRight size={14} weight="bold" />
      </motion.span>
    </button>
    <AnimatePresence>
      {expanded && (
        <motion.div
          // Height to `auto` is measured by framer, so this animates properly
          // rather than snapping. `overflow-hidden` is what makes the content
          // wipe rather than spill out during the transition.
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: 'auto', opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: 0.26, ease: [0.16, 1, 0.3, 1] }}
          className="overflow-hidden"
        >
          <div className="flex flex-col gap-2 pb-1">{children}</div>
        </motion.div>
      )}
    </AnimatePresence>
  </div>
);

// Mirrors the desktop Global Cosmetics gallery: newest first, with each
// badge's live earn status derived from its BadgeBase window.
interface GlobalBadge {
  key: string;
  setId: string;
  versionId: string;
  title: string;
  description: string;
  image: string;
  /** Precomputed newest-first rank from the badge metadata cache. */
  position: number;
  /** Parsed date_added, the authoritative newest-first key. */
  addedMs: number;
  usage: number;
  status: BadgeWindowStatus | null;
  dateInfo: string;
  moreInfo: string;
  infoUrl: string;
}

type BadgeSort = 'newest' | 'oldest' | 'available' | 'soon' | 'usage';

const BADGE_SORTS: { id: BadgeSort; label: string }[] = [
  { id: 'newest', label: 'Newest' },
  { id: 'available', label: 'Available' },
  { id: 'soon', label: 'Coming soon' },
  { id: 'usage', label: 'Most used' },
  { id: 'oldest', label: 'Oldest' },
];

// "1,234 users" -> 1234, so the usage sort has something numeric to work with.
function parseUsage(raw: string | null | undefined): number {
  if (!raw) return 0;
  const digits = raw.replace(/[^0-9]/g, '');
  return digits ? parseInt(digits, 10) : 0;
}

// How long is left to earn a campaign, at the coarsest useful resolution.
// Anything past a couple of days does not need an hour count, and anything
// under a day very much does.
function campaignEndsIn(campaign: DropCampaign): string | null {
  const end = new Date(campaign.end_at).getTime();
  if (Number.isNaN(end)) return null;
  const ms = end - Date.now();
  if (ms <= 0) return null;
  const hours = Math.floor(ms / 3_600_000);
  if (hours < 1) return `${Math.max(1, Math.round(ms / 60_000))}m left`;
  if (hours < 48) return `${hours}h left`;
  return `${Math.floor(hours / 24)}d left`;
}

function parseAdded(raw: string | null | undefined): number {
  if (!raw) return 0;
  const ms = new Date(raw).getTime();
  return Number.isNaN(ms) ? 0 : ms;
}

// A pushed badge has no scraped date_added yet, but its relay enrichment
// carries the campaign window; the window opening is an honest "how new is
// this" stand-in, and without it a fresh badge sorts as if it were ancient.
function enrichmentStartMs(meta: CachedBadgeMeta | undefined): number {
  const raw = meta?.data?.enrichment?.['starts_utc'];
  if (typeof raw !== 'string') return 0;
  const ms = new Date(raw).getTime();
  return Number.isNaN(ms) ? 0 : ms;
}

// Version ids are numeric strings in practice; compare them as numbers so
// "10" beats "9", falling back to string order for anything exotic.
function versionRank(id: string): number {
  const n = parseInt(id, 10);
  return Number.isNaN(n) ? 0 : n;
}

// Split a badge blurb into its parts so each gets its own treatment instead of
// one wall of text: the earn prose, the event window line, and any
// eligibility caveat (the "Prime subs don't count" class of parenthetical).
const WINDOW_LINE_RE = /^\s*(?:event duration|available)\s*:?\s*(.+)$/im;
const CAVEAT_RE = /\(([^)]*(?:don't count|do not count|not eligible|excluded|doesn't count)[^)]*)\)/i;

function splitBadgeBlurb(text: string): {
  prose: string;
  window: string | null;
  caveat: string | null;
} {
  if (!text) return { prose: '', window: null, caveat: null };
  let rest = text;

  const win = rest.match(WINDOW_LINE_RE);
  const window = win ? win[1].trim() : null;
  if (win) rest = rest.replace(win[0], '');

  const cav = rest.match(CAVEAT_RE);
  const caveat = cav ? cav[1].trim() : null;
  if (cav) rest = rest.replace(cav[0], '');

  const prose = rest
    .replace(/\s{2,}/g, ' ')
    .replace(/\s+([.,])/g, '$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  return { prose, window, caveat };
}
interface GlobalBadgeVersion {
  id?: string;
  title?: string;
  description?: string;
  image_url_2x?: string;
  image_url_4x?: string;
}
interface GlobalBadgeSet {
  set_id?: string;
  versions?: GlobalBadgeVersion[];
}
interface GlobalBadgeResponse {
  data?: GlobalBadgeSet[];
}
interface CachedBadgeMeta {
  data?: {
    date_added?: string | null;
    usage_stats?: string | null;
    more_info?: string | null;
    enrichment?: Record<string, unknown> | null;
    info_url?: string;
  };
  position?: number;
}

export const RewardsScreen: React.FC = () => {
  const addToast = useAppStore((s) => s.addToast);
  const currentUser = useAppStore((s) => s.currentUser);
  const [tab, setTab] = useState<RewardsTab>('drops');

  // Arriving from a tap on the live drop progress: land on Drops, scroll that
  // campaign into view and flash it, so the jump obviously ends somewhere rather
  // than dumping you at the top of a list to hunt.
  const focusDropCampaignId = useMobileNavStore((s) => s.focusDropCampaignId);
  const clearDropFocus = useMobileNavStore((s) => s.clearDropFocus);
  const openBrowseCategory = useMobileNavStore((s) => s.openBrowseCategory);
  // Named apart from this screen's own `setTab`, which switches drops/badges.
  const setNavTab = useMobileNavStore((s) => s.setTab);
  const campaignRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const [flashedCampaign, setFlashedCampaign] = useState<string | null>(null);
  // Tapping a campaign opens every reward in it, the way the desktop Drops
  // Center does. The card alone only says "2/5", which tells you nothing about
  // WHAT the remaining three are.
  // One sheet serves both lists. An inventory entry knows how many of its drops
  // you have already claimed; a campaign you have not started has no such entry,
  // so `claimed` is null and the sheet says how many rewards it holds instead.
  const [campaignDetail, setCampaignDetail] = useState<{
    campaign: DropCampaign;
    claimed: number | null;
    total: number;
  } | null>(null);
  const [campaigns, setCampaigns] = useState<DropCampaign[]>([]);
  const [dropsQuery, setDropsQuery] = useState('');
  // Participating-channels list inside the campaign sheet. Stamped with the
  // campaign it was opened for and compared during render, so opening a
  // different campaign starts closed without an effect to reset it.
  const [channelsOpenFor, setChannelsOpenFor] = useState<string | null>(null);
  const startStream = useAppStore((s) => s.startStream);
  const [globalBadges, setGlobalBadges] = useState<GlobalBadge[]>([]);
  const [ownedTitles, setOwnedTitles] = useState<Set<string>>(new Set());
  const [badgesLoading, setBadgesLoading] = useState(false);
  const [badgeSort, setBadgeSort] = useState<BadgeSort>('newest');
  const [badgeDetail, setBadgeDetail] = useState<GlobalBadge | null>(null);
  const [metaProgress, setMetaProgress] = useState(0);
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [inventory, setInventory] = useState<InventoryResponse | null>(null);
  const [deviceCode, setDeviceCode] = useState<DropsDeviceCodeInfo | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [copied, setCopied] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const ok = await invoke<boolean>('is_drops_authenticated');
      setAuthed(ok);
      if (ok) {
        const inv = await invoke<InventoryResponse>('get_drops_inventory').catch(() => null);
        setInventory(inv);
        // Everything currently running, not just what you have already started.
        // The inventory only lists a campaign once it has progress, so on its
        // own it can never answer "what could I be earning right now". The
        // backend caches this for a few minutes, so asking on load and on
        // pull-to-refresh is cheap.
        const active = await invoke<DropCampaign[]>('get_active_drop_campaigns').catch(() => []);
        setCampaigns(active ?? []);
      }
    } catch (err) {
      Logger.warn('[Rewards] load failed:', err);
      setAuthed(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Consume a pending focus once the campaign has actually rendered. Runs off
  // `inventory` too because the tap usually lands before the list has loaded,
  // and scrolling to an element that does not exist yet does nothing.
  useEffect(() => {
    if (!focusDropCampaignId) return;
    if (tab !== 'drops') setTab('drops');
    const el = campaignRefs.current[focusDropCampaignId];
    if (!el) return;
    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    setFlashedCampaign(focusDropCampaignId);
    clearDropFocus();
    const t = setTimeout(() => setFlashedCampaign(null), 1800);
    return () => clearTimeout(t);
  }, [focusDropCampaignId, inventory, tab, clearDropFocus]);

  // The GLOBAL Twitch badge collection (every badge currently available),
  // with the ones you already own marked. Same sources the desktop badge wall
  // uses: the cached global badge set, plus your earned set for ownership.
  const loadBadges = useCallback(async () => {
    setBadgesLoading(true);
    try {
      let global = await invoke<GlobalBadgeResponse | null>('get_cached_global_badges');
      if (!global?.data?.length) {
        await invoke('prefetch_global_badges').catch(() => {});
        global = await invoke<GlobalBadgeResponse | null>('get_cached_global_badges');
      }

      // Badge metadata (earn window + newest-first position) comes from the
      // universal cache in one batch, keyed exactly as the desktop gallery
      // keys it.
      let meta: Record<string, CachedBadgeMeta> = {};
      try {
        meta =
          (await invoke<Record<string, CachedBadgeMeta>>('get_all_universal_cached_items', {
            cacheType: 'badge',
          })) ?? {};
      } catch (err) {
        Logger.warn('[Rewards] badge metadata cache unavailable:', err);
      }

      const build = (metaMap: Record<string, CachedBadgeMeta>): GlobalBadge[] => {
        // Keyed by title. The same badge genuinely repeats across sets (keep
        // the first), but a REVISION arrives as a higher version id in the
        // SAME set with the same title, and it must replace the original:
        // first-wins here is how the gallery kept rendering a retired
        // revision's window ("Ended") for a badge that had just relaunched,
        // and why pull-to-refresh appeared to do nothing.
        const byTitle = new Map<string, GlobalBadge>();
        for (const set of global?.data ?? []) {
          for (const v of set.versions ?? []) {
            const image = v.image_url_4x || v.image_url_2x;
            if (!v.title || !image || !set.set_id || !v.id) continue;
            const cached = metaMap[`metadata:${set.set_id}-v${v.id}`];
            const entry: GlobalBadge = {
              key: `${set.set_id}-${v.id}`,
              setId: set.set_id,
              versionId: v.id,
              title: v.title,
              description: v.description ?? '',
              image,
              position:
                typeof cached?.position === 'number' ? cached.position : Number.MAX_SAFE_INTEGER,
              addedMs: parseAdded(cached?.data?.date_added) || enrichmentStartMs(cached),
              usage: parseUsage(cached?.data?.usage_stats),
              status: deriveBadgeStatus(cached?.data?.more_info, cached?.data?.enrichment),
              dateInfo: formatBadgeDateInfo(cached?.data?.more_info),
              moreInfo: cached?.data?.more_info ?? '',
              infoUrl: cached?.data?.info_url ?? '',
            };
            const prev = byTitle.get(v.title);
            if (
              !prev ||
              (prev.setId === entry.setId &&
                versionRank(entry.versionId) > versionRank(prev.versionId))
            ) {
              byTitle.set(v.title, entry);
            }
          }
        }
        return [...byTitle.values()];
      };

      setGlobalBadges(build(meta));

      const uid = currentUser?.user_id;
      const login = currentUser?.login || currentUser?.username;
      if (uid && login) {
        const mine = await getAllUserBadgesWithEarned(uid, login, uid, login);
        setOwnedTitles(new Set((mine.earnedBadges ?? []).map((b) => b.title)));
      }
      setBadgesLoading(false);

      // Mobile had never populated the badge metadata cache, which is why the
      // gallery had almost no dates or earn windows to sort by. Fetch what is
      // missing in batches (same commands the desktop gallery uses), then
      // rebuild from the refreshed cache.
      try {
        const missing = await invoke<[string, string][]>('get_badges_missing_metadata');
        if (missing.length > 0) {
          setMetaProgress(missing.length);
          const batchSize = 5;
          for (let i = 0; i < missing.length; i += batchSize) {
            await Promise.allSettled(
              missing.slice(i, i + batchSize).map(([setId, version]) =>
                invoke('fetch_badge_metadata', { badgeSetId: setId, badgeVersion: version }),
              ),
            );
            setMetaProgress(Math.max(0, missing.length - (i + batchSize)));
          }
          const refreshed =
            (await invoke<Record<string, CachedBadgeMeta>>('get_all_universal_cached_items', {
              cacheType: 'badge',
            })) ?? {};
          setGlobalBadges(build(refreshed));
        }
      } catch (err) {
        Logger.warn('[Rewards] badge metadata backfill failed:', err);
      } finally {
        setMetaProgress(0);
      }
      return;
    } catch (err) {
      Logger.warn('[Rewards] badge load failed:', err);
    } finally {
      setBadgesLoading(false);
    }
  }, [currentUser?.user_id, currentUser?.login, currentUser?.username]);

  useEffect(() => {
    if (tab === 'badges' && globalBadges.length === 0) void loadBadges();
  }, [tab, globalBadges.length, loadBadges]);

  // Relay pushed a badge (or corrected one): the Rust side has already merged
  // the global cache and stored the enrichment, so re-reading surfaces the new
  // tile with its real window. Desktop's BadgesOverlay has had this listener
  // all along; without it the phone's gallery only ever changed on remount.
  useEffect(() => {
    const unlisten = listen('badge-metadata-amended', () => {
      void loadBadges();
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [loadBadges]);

  const connect = async () => {
    setConnecting(true);
    setConnectError(null);
    try {
      const info = await invoke<DropsDeviceCodeInfo>('start_drops_device_flow');
      setDeviceCode(info);
      // Copy the code up front, then authorize in the SAME in-app WebView the
      // main Twitch login uses (no external browser). The overlay covers the
      // app, so the code must already be on the clipboard when it opens.
      try {
        await navigator.clipboard.writeText(info.user_code);
      } catch {
        /* the code card stays visible behind the overlay regardless */
      }
      await invoke('open_mobile_login', { url: info.verification_uri }).catch(() => {});
      await invoke('poll_drops_token', {
        deviceCode: info.device_code,
        interval: info.interval,
        expiresIn: info.expires_in,
      });
      setDeviceCode(null);
      addToast('Drops connected!', 'success');
      // Trust the successful poll like desktop does; load() then fills the
      // inventory (and the backend check now agrees post-connect).
      setAuthed(true);
      await load();
    } catch (err) {
      // Surface the real backend reason (expired code, denied, network) instead
      // of a generic failure, so a stuck connect is diagnosable from the phone.
      const reason = err instanceof Error ? err.message : String(err);
      Logger.error('[Rewards] drops connect failed:', err);
      setConnectError(reason);
      addToast(`Drops connection failed: ${reason}`, 'error');
      setDeviceCode(null);
    } finally {
      await invoke('close_mobile_login').catch(() => {});
      setConnecting(false);
    }
  };

  const copyCode = async () => {
    if (!deviceCode) return;
    try {
      await navigator.clipboard.writeText(deviceCode.user_code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* the code stays visible regardless */
    }
  };

  const availableCount = globalBadges.filter(
    (b) => b.status === 'available' && !ownedTitles.has(b.title),
  ).length;

  // Same ordering rules as the desktop gallery: prefer the precomputed
  // positions when the cache has them broadly, otherwise fall back to
  // date_added, with a stable key as the final tiebreak.
  const sortedBadges = useMemo(() => {
    const withPos = globalBadges.filter((b) => b.position !== Number.MAX_SAFE_INTEGER).length;
    const usePositions = withPos >= globalBadges.length * 0.9 && globalBadges.length > 0;
    // Desktop rule, kept faithfully: positions only order badges that BOTH
    // have one. A badge without a position while positions are in broad use is
    // one the ranker has not seen yet, i.e. brand new, and comparing it by
    // position sent it to the very bottom of Newest; dates place it honestly.
    const byNewest = (a: GlobalBadge, b: GlobalBadge) => {
      if (
        usePositions &&
        a.position !== Number.MAX_SAFE_INTEGER &&
        b.position !== Number.MAX_SAFE_INTEGER
      ) {
        return a.position - b.position || a.key.localeCompare(b.key);
      }
      return b.addedMs - a.addedMs || a.key.localeCompare(b.key);
    };

    return [...globalBadges].sort((a, b) => {
      switch (badgeSort) {
        case 'oldest':
          return byNewest(b, a);
        case 'available': {
          const rank = (x: GlobalBadge) => (x.status === 'available' ? 1 : 0);
          return rank(b) - rank(a) || byNewest(a, b);
        }
        case 'soon': {
          const rank = (x: GlobalBadge) => (x.status === 'coming-soon' ? 1 : 0);
          return rank(b) - rank(a) || byNewest(a, b);
        }
        case 'usage':
          return b.usage - a.usage || byNewest(a, b);
        default:
          return byNewest(a, b);
      }
    });
  }, [globalBadges, badgeSort]);

  const inProgress = (inventory?.items ?? []).filter(
    (i: InventoryItem) => i.status === 'Active' || i.drops_in_progress > 0,
  );
  const completed = inventory?.completed_drops ?? [];

  // Everything running that is not already listed above, soonest to end first.
  // A campaign you have progress on appears in both feeds otherwise, and the
  // one with a progress bar on it is the more useful of the two.
  const available = useMemo(() => {
    const started = new Set(inProgress.map((i) => i.campaign.id));
    return campaigns
      .filter((c) => !started.has(c.id))
      .sort((a, b) => new Date(a.end_at).getTime() - new Date(b.end_at).getTime());
    // inProgress is rebuilt every render from `inventory`, so depend on that.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [campaigns, inventory]);

  // Grouped by game, and filtered by the search box.
  //
  // A busy game routinely runs several campaigns at once, and flat lists showed
  // them as the same game's art and name repeated down the screen with no
  // indication they belonged together. One heading per game with its campaigns
  // beneath says what the list actually contains.
  const q = dropsQuery.trim().toLowerCase();
  const matchesQuery = (game: string, name: string) =>
    !q || game.toLowerCase().includes(q) || name.toLowerCase().includes(q);

  const inProgressGroups = groupByGame(
    inProgress.filter((i) => matchesQuery(i.campaign.game_name || '', i.campaign.name || '')),
    (i) => i.campaign.game_name || '',
    (i) => i.campaign.image_url || '',
  );
  const availableGroups = groupByGame(
    available.filter((c) => matchesQuery(c.game_name || '', c.name || '')),
    (c) => c.game_name || '',
    (c) => c.image_url || '',
  );

  // Which game groups are open. Keyed by section too, since the same game can
  // appear under both In progress and Available and they open independently.
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());
  const toggleGroup = (key: string) =>
    setOpenGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  // A search opens everything it matched. Making someone tap again to see what
  // their own search found would be a strange thing to ask.
  const isGroupOpen = (key: string) => !!q || openGroups.has(key);

  // Re-settles on each new search, so results arrive the same way the rest of
  // the app's lists do. Refreshing the same view keeps its key and stays put.
  // The headings are what settle, since the campaigns start collapsed.
  const dropsSettled = useSettleIn(
    authed === true && inProgressGroups.length + availableGroups.length > 0,
    `drops:${q}`,
  );
  const badgesSettled = useSettleIn(sortedBadges.length > 0, `badges:${badgeSort}`);

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="px-4 pt-3 pb-2 shrink-0">
        <h1 className="text-xl font-bold text-textPrimary mb-2.5">Rewards</h1>
        <div className="flex gap-1">
          {(['drops', 'badges'] as RewardsTab[]).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`px-3.5 py-1.5 rounded-full text-sm transition-colors ${
                tab === t ? 'glass-button-static text-textPrimary font-semibold' : 'text-textMuted'
              }`}
            >
              {t === 'drops' ? 'Drops' : 'Badges'}
            </button>
          ))}
        </div>
      </div>
      <PullToRefresh onRefresh={tab === 'drops' ? load : loadBadges}>
        <div className={`px-4 sn-tabbar-clearance ${tab === 'badges' ? '' : 'hidden'}`}>
          {badgesLoading && globalBadges.length === 0 ? (
            <div className="py-10 text-center text-sm text-textMuted">Loading badges…</div>
          ) : globalBadges.length === 0 ? (
            <div className="py-10 text-center text-sm text-textMuted">
              No badges available right now.
            </div>
          ) : (
            <>
              <div className="flex items-center gap-2 pt-1 pb-2">
                <span className="text-[12px] text-textMuted">
                  {ownedTitles.size} of {globalBadges.length} collected
                </span>
                {availableCount > 0 && (
                  <span className="text-[12px] text-success font-medium">
                    {availableCount} available now
                  </span>
                )}
                {metaProgress > 0 && (
                  <span className="ml-auto text-[11.5px] text-textMuted">
                    {metaProgress} to sync
                  </span>
                )}
              </div>
              {/* Sort options, mirroring the desktop gallery's set. */}
              <div className="flex gap-1 overflow-x-auto pb-2 -mx-1 px-1">
                {BADGE_SORTS.map((s) => (
                  <button
                    key={s.id}
                    onClick={() => setBadgeSort(s.id)}
                    className={`shrink-0 px-3 py-1 rounded-full text-[12.5px] transition-colors ${
                      badgeSort === s.id
                        ? 'glass-button-static text-textPrimary font-semibold'
                        : 'text-textMuted'
                    }`}
                  >
                    {s.label}
                  </button>
                ))}
              </div>
              <div className="grid grid-cols-3 sm:grid-cols-5 lg:grid-cols-7 gap-2">
                {sortedBadges.map((badge, bi) => {
                  const owned = ownedTitles.has(badge.title);
                  const available = badge.status === 'available';
                  const comingSoon = badge.status === 'coming-soon';
                  return (
                    <SettleIn key={badge.key} index={bi} settled={badgesSettled}>
                    <button
                      onClick={() => setBadgeDetail(badge)}
                      className={`glass-panel p-2 flex flex-col items-center gap-1.5 relative active:opacity-80 w-full h-full ${
                        available && !owned ? 'ring-1 ring-success/70' : ''
                      } ${owned ? 'ring-1 ring-accent/60' : ''}`}
                    >
                      <img
                        src={badge.image}
                        alt=""
                        loading="lazy"
                        className={`w-11 h-11 object-contain ${
                          owned || available ? '' : 'opacity-45 grayscale'
                        }`}
                        draggable={false}
                      />
                      <span
                        className={`text-[10.5px] text-center line-clamp-2 leading-tight ${
                          owned || available ? 'text-textPrimary' : 'text-textMuted'
                        }`}
                      >
                        {badge.title}
                      </span>
                      {/* Live earn status: owned wins, then the window state. */}
                      {owned ? (
                        <span className="text-[9.5px] font-semibold text-accent leading-none">
                          OWNED
                        </span>
                      ) : available ? (
                        <span className="text-[9.5px] font-semibold text-success leading-none">
                          AVAILABLE
                        </span>
                      ) : comingSoon ? (
                        <span className="text-[9.5px] font-semibold text-warning leading-none">
                          SOON
                        </span>
                      ) : (
                        <span className="text-[9.5px] text-textMuted leading-none">
                          {/* Only a KNOWN-over window says so. A null status is
                              "no window known" (permanent badges, and live ones
                              whose dates the relay has not confirmed yet), and
                              claiming ENDED there is a confident lie: the Hulk
                              badge launched with "duration unconfirmed" prose
                              and spent a day marked ended before it started.
                              The detail sheet has always gated on expired. */}
                          {badge.status === 'expired' ? 'ENDED' : ''}
                        </span>
                      )}
                    </button>
                    </SettleIn>
                  );
                })}
              </div>
            </>
          )}
        </div>
        <div className={`px-4 sn-tabbar-clearance ${tab === 'drops' ? '' : 'hidden'}`}>
          {authed === false && (
            <div className="glass-panel p-4 mt-2">
              <div className="flex items-center gap-2 mb-1.5">
                <Gift size={18} className="text-accent" />
                <span className="text-[15px] font-semibold text-textPrimary">Twitch Drops</span>
              </div>
              <p className="text-[13px] text-textSecondary mb-3 leading-relaxed">
                Connect drops to track campaign progress and earn while you watch.
              </p>
              {deviceCode ? (
                <div className="text-center">
                  <button
                    onClick={copyCode}
                    className="glass-input w-full py-3 font-mono text-2xl tracking-[0.3em] text-textPrimary"
                  >
                    {deviceCode.user_code}
                  </button>
                  <div className="text-[12px] text-textMuted mt-1.5 min-h-[16px]">
                    {copied ? 'Copied' : 'Code copied. Paste it on the Twitch page that opens.'}
                  </div>
                  <div className="flex items-center justify-center gap-1.5 text-[12.5px] text-textMuted mt-2">
                    <span className="w-2 h-2 rounded-full bg-accent animate-pulse" />
                    Waiting for authorization…
                  </div>
                </div>
              ) : (
                <>
                  <button
                    onClick={() => void connect()}
                    disabled={connecting}
                    className="glass-button sn-touch w-full text-[14px] font-semibold text-textPrimary disabled:opacity-60 flex items-center justify-center gap-1.5"
                  >
                    Connect drops
                    <ArrowSquareOut size={15} />
                  </button>
                  {connectError && (
                    <div className="mt-2 text-[12px] text-error leading-snug break-words">
                      {connectError}
                    </div>
                  )}
                </>
              )}
            </div>
          )}

          {authed && (
            <>
              {/* Searching by game, since that is how anyone thinks about
                  drops. Campaign names are matched too, because a lot of them
                  read like event names and are what someone actually remembers. */}
              <div className="relative mt-1 mb-1">
                <MagnifyingGlass
                  size={16}
                  className="absolute left-3 top-1/2 -translate-y-1/2 text-textMuted z-10"
                />
                <input
                  value={dropsQuery}
                  onChange={(e) => setDropsQuery(e.target.value)}
                  placeholder="Search games or campaigns"
                  className="glass-input w-full rounded-lg pl-9 pr-9 py-2.5 text-[14px] text-textPrimary placeholder:text-textMuted outline-none"
                />
                {dropsQuery && (
                  <button
                    onClick={() => setDropsQuery('')}
                    aria-label="Clear search"
                    className="absolute right-2 top-1/2 -translate-y-1/2 p-1.5 text-textMuted z-10"
                  >
                    <X size={15} />
                  </button>
                )}
              </div>

              <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mt-2 mb-1.5">
                In progress
              </div>
              {inProgressGroups.length === 0 ? (
                <div className="glass-panel p-4 text-[13px] text-textMuted">
                  {q
                    ? 'No drops in progress match that search.'
                    : 'No drop campaigns in progress. Watch a drops-enabled stream to start earning.'}
                </div>
              ) : (
                inProgressGroups.map((group, gIdx) => (
                  <SettleIn key={group.game} index={gIdx} settled={dropsSettled}>
                    <GameGroupSection
                      art={group.art}
                      game={group.game}
                      count={group.items.length}
                      expanded={isGroupOpen(`progress:${group.game}`)}
                      onToggle={() => toggleGroup(`progress:${group.game}`)}
                    >
                      {group.items.map((item) => (
                        <React.Fragment key={item.campaign.id}>
                    <div
                      ref={(el) => {
                        campaignRefs.current[item.campaign.id] = el;
                      }}
                      role="button"
                      tabIndex={0}
                      onClick={() =>
                        setCampaignDetail({
                          campaign: item.campaign,
                          claimed: item.claimed_drops,
                          total: item.total_drops,
                        })
                      }
                      className={`glass-panel p-2.5 flex gap-2.5 transition-shadow duration-500 active:opacity-80 ${
                        flashedCampaign === item.campaign.id ? 'ring-2 ring-accent' : ''
                      }`}
                    >
                      {/* No art on the card. `image_url` is the GAME's box art,
                          so inside a group it is the heading's image repeated
                          once per row, eating a third of the width to say the
                          same thing six times. The name and the progress are
                          what differ between these, so they get the space. */}
                      <div className="flex-1 min-w-0 flex flex-col justify-center">
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-[13.5px] font-medium text-textPrimary truncate">
                            {item.campaign.name}
                          </span>
                          <span className="text-[12px] text-textMuted shrink-0">
                            {item.claimed_drops}/{item.total_drops}
                          </span>
                        </div>
                        {/* No game name here: the heading above the group
                            already says it, and repeating it on every card is
                            what made these lists read as noise. */}
                        <div className="h-1.5 rounded-full bg-surface overflow-hidden mt-1.5">
                          <div
                            className="h-full rounded-full bg-accent transition-[width]"
                            style={{ width: `${Math.min(100, item.progress_percentage)}%` }}
                          />
                        </div>
                      </div>
                    </div>
                        </React.Fragment>
                      ))}
                    </GameGroupSection>
                  </SettleIn>
                ))
              )}

              {availableGroups.length > 0 && (
                <>
                  <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mt-4 mb-1.5">
                    Available now
                  </div>
                  {availableGroups.map((group, gIdx) => (
                    <SettleIn
                      key={group.game}
                      index={inProgressGroups.length + gIdx}
                      settled={dropsSettled}
                    >
                      <GameGroupSection
                        art={group.art}
                        game={group.game}
                        count={group.items.length}
                        expanded={isGroupOpen(`available:${group.game}`)}
                        onToggle={() => toggleGroup(`available:${group.game}`)}
                      >
                        {group.items.map((campaign) => {
                          const rewards = campaign.time_based_drops?.length ?? 0;
                          // What the row is FOR is showing the reward, so show
                          // the reward: the first tier's benefit art, the same
                          // image the detail sheet lists below.
                          //
                          // `campaign.image_url` was being used here and is not
                          // that. It is the GAME's box art (desktop feeds the
                          // very same field into getOrCreateGame), and it arrives
                          // as a `{width}x{height}` template. Handed to an img
                          // unresolved, Twitch's CDN answers with its generic
                          // grey box art, which is why every one of these tiles
                          // turned into the same placeholder gamepad. It stays
                          // as the fallback, but resolved through gameBoxArt
                          // this time.
                          const rewardArt =
                            campaign.time_based_drops?.[0]?.benefit_edges?.[0]?.image_url;
                          const art =
                            rewardArt ||
                            (campaign.image_url ? gameBoxArt(campaign.image_url, 144, 192) : '');
                          return (
                            <React.Fragment key={campaign.id}>
                              <div
                                role="button"
                                tabIndex={0}
                                onClick={() =>
                                  setCampaignDetail({ campaign, claimed: null, total: rewards })
                                }
                                className="glass-panel p-2.5 flex gap-2.5 active:opacity-80"
                              >
                                {art ? (
                                  <img
                                    src={art}
                                    alt=""
                                    loading="lazy"
                                    draggable={false}
                                    className="w-[52px] h-[52px] shrink-0 object-cover rounded-md ring-1 ring-white/10"
                                  />
                                ) : (
                                  <div className="w-[52px] h-[52px] shrink-0 rounded-md bg-surface flex items-center justify-center">
                                    <Gift size={18} className="text-textMuted" />
                                  </div>
                                )}
                                <div className="flex-1 min-w-0 flex flex-col justify-center">
                                  <div className="flex items-center gap-2">
                                    <span className="text-[13.5px] font-medium text-textPrimary truncate">
                                      {campaign.name}
                                    </span>
                                    {!campaign.is_account_connected && (
                                      <Warning size={14} className="text-warning shrink-0" />
                                    )}
                                  </div>
                                  {/* Game name lives on the group heading now. */}
                                  <div className="text-[11.5px] text-textMuted mt-1">
                                    {rewards} {rewards === 1 ? 'reward' : 'rewards'}
                                    {campaignEndsIn(campaign)
                                      ? ` · ${campaignEndsIn(campaign)}`
                                      : ''}
                                  </div>
                                </div>
                              </div>
                            </React.Fragment>
                          );
                        })}
                      </GameGroupSection>
                    </SettleIn>
                  ))}
                </>
              )}

              {completed.length > 0 && (
                <>
                  <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mt-4 mb-1.5">
                    Recently earned
                  </div>
                  <div className="flex flex-col gap-2">
                    {completed.slice(0, 10).map((drop) => (
                      <div key={drop.id} className="glass-panel p-2.5 flex items-center gap-2.5">
                        {drop.image_url ? (
                          <img
                            src={drop.image_url}
                            alt=""
                            className="w-9 h-9 rounded object-cover shrink-0"
                            draggable={false}
                          />
                        ) : (
                          <Gift size={20} className="text-accent shrink-0" />
                        )}
                        <div className="flex-1 min-w-0">
                          <div className="text-[13.5px] text-textPrimary truncate">{drop.name}</div>
                          {drop.game_name && (
                            <div className="text-[12px] text-textMuted truncate">{drop.game_name}</div>
                          )}
                        </div>
                        <CheckCircle size={16} className="text-success shrink-0" />
                      </div>
                    ))}
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </PullToRefresh>

      {/* Badge detail: art, status, when it was added, how it is earned. */}
      <MobileSheet
        open={!!badgeDetail}
        onClose={() => setBadgeDetail(null)}
        maxHeightFraction={0.66}
      >
        {badgeDetail && (
          <div className="flex flex-col items-center text-center pb-1">
            <img
              src={badgeDetail.image}
              alt=""
              className="w-20 h-20 object-contain mb-2.5"
              draggable={false}
            />
            <div className="text-[16px] font-semibold text-textPrimary">{badgeDetail.title}</div>
            <div className="mt-1 mb-2.5">
              {ownedTitles.has(badgeDetail.title) ? (
                <span className="text-[11px] font-semibold text-accent">OWNED</span>
              ) : badgeDetail.status === 'available' ? (
                <span className="text-[11px] font-semibold text-success">AVAILABLE NOW</span>
              ) : badgeDetail.status === 'coming-soon' ? (
                <span className="text-[11px] font-semibold text-warning">COMING SOON</span>
              ) : badgeDetail.status === 'expired' ? (
                <span className="text-[11px] font-semibold text-textMuted">NO LONGER EARNABLE</span>
              ) : null}
            </div>
            {/* ONE body, split into parts. `dateInfo` is the full more_info
                blurb with its dates localized (not a short date label), so it
                is the source for all three pieces below. */}
            {(() => {
              const { prose, window, caveat } = splitBadgeBlurb(
                badgeDetail.dateInfo || badgeDetail.description,
              );
              return (
                <>
                  {prose && (
                    <p className="text-[13px] text-textSecondary leading-relaxed whitespace-pre-line text-left w-full">
                      {prose}
                    </p>
                  )}
                  {caveat && (
                    <div className="mt-3 w-full flex items-start gap-2 rounded-lg px-3 py-2 bg-amber-500/10 border border-amber-500/25">
                      <Warning size={14} weight="fill" className="text-amber-400 shrink-0 mt-px" />
                      <span className="text-[12.5px] text-amber-300/90 leading-snug text-left">
                        {caveat}
                      </span>
                    </div>
                  )}
                  {window && (
                    <div className="mt-3 w-full glass-tile rounded-lg px-3 py-2.5 flex items-center gap-2.5">
                      <CalendarBlank size={16} className="text-accent shrink-0" />
                      <div className="min-w-0 text-left">
                        <div className="text-[10.5px] font-semibold uppercase tracking-wide text-textMuted leading-none mb-1">
                          Event duration
                        </div>
                        <div className="text-[12.5px] text-textPrimary leading-snug">{window}</div>
                      </div>
                    </div>
                  )}
                </>
              );
            })()}
            {badgeDetail.usage > 0 && (
              <p className="text-[12px] text-textMuted mt-3 self-center">
                {badgeDetail.usage.toLocaleString()} users have this badge
              </p>
            )}
            {badgeDetail.infoUrl && (
              <button
                onClick={() => {
                  void invoke('open_browser_url', { url: badgeDetail.infoUrl }).catch(() => {});
                }}
                className="glass-button sn-touch mt-4 w-full text-[13.5px] font-semibold text-textPrimary"
              >
                More info
              </button>
            )}
          </div>
        )}
      </MobileSheet>

      {/* Every reward in one campaign, in the order you earn them. */}
      <MobileSheet
        open={!!campaignDetail}
        onClose={() => setCampaignDetail(null)}
        title={campaignDetail?.campaign.name}
        maxHeightFraction={0.8}
      >
        {campaignDetail && (
          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2 text-[12.5px] text-textMuted">
              {campaignDetail.campaign.game_name && (
                <span className="truncate">{campaignDetail.campaign.game_name}</span>
              )}
              <span className="ml-auto shrink-0">
                {campaignDetail.claimed !== null
                  ? `${campaignDetail.claimed}/${campaignDetail.total} claimed`
                  : `${campaignDetail.total} ${campaignDetail.total === 1 ? 'reward' : 'rewards'}`}
              </span>
            </div>

            {/* Twitch will not credit watch time until the game account is
                linked, so saying this up front is the difference between a
                wasted evening and a working one. */}
            {!campaignDetail.campaign.is_account_connected && (
              <button
                onClick={() => {
                  const url = campaignDetail.campaign.account_link;
                  if (!url) return;
                  void openExternal(url).then((ok) => {
                    if (!ok) addToast('Could not open the account link page.', 'error');
                  });
                }}
                disabled={!campaignDetail.campaign.account_link}
                className="glass-panel p-2.5 flex items-center gap-2 text-left disabled:opacity-70"
              >
                <Warning size={16} className="text-warning shrink-0" />
                <span className="flex-1 text-[12.5px] text-textSecondary leading-snug">
                  Your game account is not linked, so this campaign will not earn yet.
                </span>
                {campaignDetail.campaign.account_link && (
                  <ArrowSquareOut size={14} className="text-textMuted shrink-0" />
                )}
              </button>
            )}

            {/* Where it can be earned. An ACL campaign only credits on these
                channels, so it is the whole plan for the evening rather than
                trivia; a category-wide one lists none and needs no list.

                Collapsed, and a list rather than prose. These run to dozens of
                names, and joining them with commas produced a paragraph nobody
                could read or act on. Each name is now a row you can tap to go
                and watch it, which is the only thing anyone wanted from this
                list in the first place. */}
            {campaignDetail.campaign.allowed_channels?.length > 0 && (() => {
              const channelsOpen = channelsOpenFor === campaignDetail.campaign.id;
              return (
              <div>
                <button
                  onClick={() =>
                    setChannelsOpenFor(channelsOpen ? null : campaignDetail.campaign.id)
                  }
                  aria-expanded={channelsOpen}
                  className="w-full glass-panel px-3 py-2.5 flex items-center gap-2 text-left active:opacity-80"
                >
                  <span className="text-[12.5px] text-textSecondary flex-1">
                    Earn on {campaignDetail.campaign.allowed_channels.length}{' '}
                    {campaignDetail.campaign.allowed_channels.length === 1
                      ? 'channel'
                      : 'channels'}
                  </span>
                  <motion.span
                    className="shrink-0 flex text-textMuted"
                    animate={{ rotate: channelsOpen ? 90 : 0 }}
                    transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
                  >
                    <CaretRight size={13} weight="bold" />
                  </motion.span>
                </button>
                <AnimatePresence>
                  {channelsOpen && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.26, ease: [0.16, 1, 0.3, 1] }}
                      className="overflow-hidden"
                    >
                      {/* Capped height so a campaign with eighty channels does
                          not push the rest of the sheet off the bottom. */}
                      <div className="max-h-[40vh] overflow-y-auto mt-1 flex flex-col">
                        {campaignDetail.campaign.allowed_channels.map((c) => (
                          <button
                            key={c.id || c.name}
                            onClick={() => {
                              setCampaignDetail(null);
                              setChannelsOpenFor(null);
                              // Pass the broadcaster id through. Without it
                              // startStream falls back to a channel-info lookup,
                              // and if that throws it lands on an empty user_id,
                              // which skips drops monitoring entirely and leaves
                              // the watch heartbeat pointed at whatever channel
                              // came before. A campaign's channel list is exactly
                              // where someone starts a streamer they do not
                              // follow, so the followed-streams shortcut cannot
                              // cover for it. Title and category are left blank
                              // on purpose; startStream backfills both.
                              void startStream(c.name.toLowerCase(), {
                                id: '',
                                user_id: c.id,
                                user_name: c.name,
                                user_login: c.name.toLowerCase(),
                                title: '',
                                viewer_count: 0,
                                game_name: '',
                                thumbnail_url: '',
                                started_at: new Date().toISOString(),
                              });
                            }}
                            className="px-3 py-2.5 text-left text-[13px] text-textPrimary border-b border-borderSubtle last:border-b-0 active:opacity-70"
                          >
                            {c.name}
                          </button>
                        ))}
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
              );
            })()}

            {campaignDetail.campaign.game_id && campaignDetail.campaign.game_name && (
              <button
                onClick={() => {
                  const c = campaignDetail.campaign;
                  setCampaignDetail(null);
                  openBrowseCategory({
                    id: c.game_id,
                    name: c.game_name,
                    box_art_url: c.image_url || '',
                  });
                  setNavTab('browse');
                }}
                className="glass-button rounded-lg py-2.5 text-[13px] font-medium text-textPrimary"
              >
                Find a stream
              </button>
            )}

            {[...(campaignDetail.campaign.time_based_drops || [])]
              .sort((a, b) => a.required_minutes_watched - b.required_minutes_watched)
              .map((drop) => {
                const need = drop.required_minutes_watched;
                const have = Math.min(drop.progress?.current_minutes_watched ?? 0, need);
                const claimed = !!drop.progress?.is_claimed;
                const done = claimed || (need > 0 && have >= need);
                const pct = need > 0 ? Math.min(100, (have / need) * 100) : 0;
                const benefit = drop.benefit_edges?.[0];
                return (
                  <div key={drop.id} className="flex items-center gap-2.5 py-1">
                    {benefit?.image_url ? (
                      <img
                        src={benefit.image_url}
                        alt=""
                        draggable={false}
                        className={`w-10 h-10 rounded-md object-cover shrink-0 ring-1 ring-white/10 ${
                          claimed ? '' : 'opacity-90'
                        }`}
                      />
                    ) : (
                      <div className="w-10 h-10 rounded-md bg-surface flex items-center justify-center shrink-0">
                        <Gift size={16} className="text-textMuted" />
                      </div>
                    )}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-baseline gap-2">
                        <span className="text-[13px] text-textPrimary truncate">
                          {benefit?.name || drop.name}
                        </span>
                        <span
                          className={`ml-auto text-[11px] shrink-0 tabular-nums ${
                            claimed
                              ? 'text-success'
                              : done
                                ? 'text-success font-semibold'
                                : 'text-textMuted'
                          }`}
                        >
                          {claimed ? 'Claimed' : done ? 'Ready' : `${need - have}m left`}
                        </span>
                      </div>
                      {/* A 0-minute drop is event or action based, not something
                          watch time earns, so it gets no bar to imply otherwise. */}
                      {need > 0 && (
                        <div className="h-1 rounded-full bg-surface overflow-hidden mt-1.5">
                          <div
                            className={`h-full rounded-full ${
                              done ? 'bg-success' : 'bg-accent'
                            }`}
                            style={{ width: `${claimed ? 100 : pct}%` }}
                          />
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
          </div>
        )}
      </MobileSheet>
    </div>
  );
};
