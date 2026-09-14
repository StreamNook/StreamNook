import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TwitchStream } from '../../types';
import { useAppStore } from '../../stores/AppStore';
import { useFollowsStore } from '../../stores/followsStore';
import { Logger } from '../../utils/logger';
import { makeKey, parseKey } from '../../utils/providerKey';
import { streamProvider } from '../../utils/streamProvider';
import { PROVIDER_WATCH, type ProviderId } from '../../types/providers';

/** Normalized shape so live follows and search results from any platform render
 *  through one row. `provider` absent means Twitch, matching the convention on
 *  MultiNookSlot and MultiNookPresetChannel. */
export interface ChannelItem {
  id: string;
  login: string;
  displayName: string;
  avatarUrl?: string;
  isLive: boolean;
  gameName?: string;
  source: 'following' | 'search';
  provider?: ProviderId;
}

/** The identity of a picker row. Composite ALWAYS, via makeKey, never streamKey:
 *  streamKey returns a BARE login for Twitch, so an exclude set built one way and
 *  read the other silently stops excluding anything. */
export function itemKey(it: Pick<ChannelItem, 'provider' | 'login'>): string {
  return makeKey(it.provider ?? 'twitch', it.login);
}

export const DEFAULT_AVATAR =
  'https://static-cdn.jtvnw.net/user-default-pictures-uv/75305d54-c7cc-40d1-bb9c-91c46bf27829-profile_image-70x70.png';

/** Followed-streams thumbnails are stream previews carrying {width}x{height} placeholders that
 *  won't load as-is, so prefer a real profile image and only fall back to a sized preview. */
export function resolveAvatar(profileImageUrl?: string, thumbnailUrl?: string): string | undefined {
  if (profileImageUrl) return profileImageUrl;
  if (thumbnailUrl) return thumbnailUrl.replace('{width}', '150').replace('{height}', '150');
  return undefined;
}

/** A row from the followed-live lists. Twitch's endpoint only returns live
 *  channels, but provider rows come from a poller snapshot that can hold a stale
 *  entry, so trust the row's own flag when it carries one. */
export function streamToItem(s: TwitchStream): ChannelItem {
  const provider = streamProvider(s);
  return {
    id: s.user_id,
    login: s.user_login,
    displayName: s.user_name || s.user_login,
    avatarUrl: resolveAvatar(s.profile_image_url, s.thumbnail_url),
    isLive: s.is_live ?? true,
    gameName: s.game_name,
    source: 'following',
    provider: provider === 'twitch' ? undefined : provider,
  };
}

export function resultToItem(r: TwitchStream, provider: ProviderId = 'twitch'): ChannelItem {
  const login = r.user_login || '';
  return {
    // A provider row's user_id is #[serde(default)], so it can be an empty
    // string. Falling back to the login keeps the row addressable, and keeps
    // React keys unique once itemKey wraps it.
    id: r.user_id || login,
    login,
    displayName: r.user_name || login,
    avatarUrl: resolveAvatar(r.profile_image_url, r.thumbnail_url),
    isLive: r.is_live ?? false,
    gameName: r.game_name,
    source: 'search',
    provider: provider === 'twitch' ? undefined : provider,
  };
}

/** Live follows, minus anything the caller excludes, filtered by the typed query.
 *  Exported pure so the key-space behaviour is testable without a DOM. */
export function filterFollowing(
  items: ChannelItem[],
  excludeKeys: Set<string>,
  query: string,
): ChannelItem[] {
  const kept = items.filter((it) => !excludeKeys.has(itemKey(it)));
  if (!query) return kept;
  // Substring display matching, deliberately NOT key comparison: this is an OR
  // chain that can only ADD rows, and folding it into the key space would break
  // matching on display name and category.
  return kept.filter(
    (it) =>
      it.login.toLowerCase().includes(query) ||
      it.displayName.toLowerCase().includes(query) ||
      (it.gameName || '').toLowerCase().includes(query),
  );
}

/** Search results, minus anything excluded or already shown as a live follow. */
export function filterSearch(
  items: ChannelItem[],
  excludeKeys: Set<string>,
  followingKeys: Set<string>,
): ChannelItem[] {
  const seen = new Set<string>();
  return items.filter((it) => {
    if (!it.login) return false;
    const key = itemKey(it);
    // Two platforms can return the same login, so dedupe on the composite key
    // rather than letting the second row erase the first.
    if (excludeKeys.has(key) || followingKeys.has(key) || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/** Order the merged result set so non-Twitch hits are actually visible.
 *
 *  Twitch matches fuzzily and readily returns eight rows; the other platforms
 *  return one or two at most, because Kick's "search" is an exact-slug jump and
 *  YouTube's is a live lookup. Appending them therefore buried every non-Twitch
 *  result below the fold of a short dropdown.
 *
 *  MEASURED 2026-08-28: with the platforms appended, a YouTube hit for a query
 *  that matched it landed at position NINE. A reviewer with devtools open
 *  concluded the picker searched Twitch only, and was wrong but reasonably so.
 *  A multi-platform picker that is multi-platform only if you scroll is not one.
 *
 *  Twitch keeps the lead because it is the primary platform and the common case,
 *  but only a few rows of it before the others get their turn. */
export function rankResults(twitch: ChannelItem[], others: ChannelItem[]): ChannelItem[] {
  if (others.length === 0) return twitch;
  /** Twitch rows shown before the other platforms get a turn. Small enough that
   *  the first non-Twitch hit is on screen in a short dropdown. */
  const TWITCH_LEAD = 4;
  return [...twitch.slice(0, TWITCH_LEAD), ...others, ...twitch.slice(TWITCH_LEAD)];
}

/** Read a channel the user typed by hand. Accepts the app's own `provider:channel`
 *  form, so typing `kick:xqc` adds a Kick channel instead of a Twitch channel
 *  literally named "kick:xqc".
 *
 *  parseKey alone is not enough: it reads an unrecognised prefix as a legacy
 *  Twitch login and hands back the whole string, colon included. Anything still
 *  containing a colon after parsing was therefore NOT a provider form, so it is
 *  rejected rather than guessed at. */
export function parseTypedChannel(raw: string): { provider: ProviderId; channel: string } | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const { provider, channel } = parseKey(trimmed);
  if (!channel || channel.includes(':')) return null;
  return { provider, channel };
}

export interface ChannelSearchOptions {
  /** Composite keys (makeKey) of channels the caller already has. */
  excludeKeys: Set<string>;
  /** Platforms to search. Defaults to Twitch only, so a surface that has not
   *  opted in cannot start returning channels it has no way to handle. */
  providers?: ProviderId[];
}

/**
 * Shared channel-finder logic backing the toolbar's "Add Stream" search, the
 * preset editor's channel picker, and the reminders picker. Owns: the query, the
 * debounced multi-platform search, the merged "live following + all channels"
 * lists (minus any channels the caller excludes), and the keyboard-highlight
 * index with scroll-into-view.
 *
 * The caller owns panel open/close, focus, and what happens on select. This hook
 * is purely the data + navigation layer so the surfaces stay identical.
 *
 * The options object is deliberately not a bare Set. A Set-to-Set signature
 * change type-checks silently, and moving the key space from bare login to
 * composite is exactly the kind of change that must not compile until every
 * caller has been looked at.
 */
export function useChannelSearch({ excludeKeys, providers = ['twitch'] }: ChannelSearchOptions) {
  const followedStreams = useAppStore((s) => s.followedStreams);
  const loadFollowedStreams = useAppStore((s) => s.loadFollowedStreams);
  const providerLive = useFollowsStore((s) => s.liveByKey);

  const [searchInput, setSearchInput] = useState('');
  const [isSearching, setIsSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<ChannelItem[]>([]);
  const [highlightIndex, setHighlightIndex] = useState(0);

  const listRef = useRef<HTMLDivElement>(null);
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const requestSeq = useRef(0);

  const query = searchInput.trim().toLowerCase();

  // Which non-Twitch platforms this surface searches. Gated on the declared
  // capability, so a platform without search never costs a round trip.
  const providerSig = providers.join(',');
  const searchProviders = useMemo(
    () => providers.filter((p) => p !== 'twitch' && PROVIDER_WATCH[p].search),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [providerSig],
  );
  const wantsTwitch = providers.includes('twitch');

  // Twitch follows come from Helix (AppStore). Every other platform's follows are
  // app-local and live in followsStore, because those platforms expose no
  // followed-channels API to us. Both feed one list.
  const followingItems = useMemo(() => {
    const twitch = wantsTwitch ? followedStreams.map(streamToItem) : [];
    const others = Object.values(providerLive)
      .filter((row) => row.is_live && providerSig.split(',').includes(row.provider))
      .sort((a, b) => b.viewer_count - a.viewer_count)
      .map(streamToItem);
    return filterFollowing([...twitch, ...others], excludeKeys, query);
  }, [followedStreams, providerLive, providerSig, wantsTwitch, excludeKeys, query]);

  const searchItems = useMemo(() => {
    const followingKeys = new Set(followingItems.map(itemKey));
    return filterSearch(searchResults, excludeKeys, followingKeys);
  }, [searchResults, followingItems, excludeKeys]);

  // Flat list backing keyboard navigation (following first, then search).
  const visibleItems = useMemo(() => [...followingItems, ...searchItems], [followingItems, searchItems]);

  // Reset the highlight whenever the result set changes shape.
  useEffect(() => {
    setHighlightIndex(0);
  }, [query, followingItems.length, searchItems.length]);

  // Keep the highlighted row scrolled into view during keyboard navigation.
  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-idx="${highlightIndex}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [highlightIndex]);

  // Debounced search across every requested platform. The live list above stays
  // instant and is never gated on this.
  useEffect(() => {
    if (!query) {
      setSearchResults([]);
      setIsSearching(false);
      if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current);
      return;
    }

    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current);

    setIsSearching(true);
    searchTimeoutRef.current = setTimeout(async () => {
      const seq = ++requestSeq.current;
      const raw = searchInput.trim();
      // Every platform settles INDEPENDENTLY. Kick's client times out at 12s and
      // YouTube's at 15s, and both fail often enough (Cloudflare, expired
      // session) that an all-or-nothing await would blank the whole picker,
      // including the Twitch results that already came back.
      const [twitchRows, ...providerRows] = await Promise.all([
        wantsTwitch
          ? (invoke('search_channels', { query: raw }) as Promise<TwitchStream[]>).catch((err) => {
              Logger.error('channel search failed:', err);
              return [] as TwitchStream[];
            })
          : Promise.resolve([] as TwitchStream[]),
        ...searchProviders.map((p) =>
          invoke<{ streams: TwitchStream[] }>('provider_search', { provider: p, query: raw })
            .then((page) => (page?.streams ?? []).map((r) => resultToItem(r, p)))
            // LOGGED, not swallowed. This catch only fires when the call actually
            // REJECTS, never on an empty result, so it cannot become per-keystroke
            // noise for a platform that simply found nothing.
            //
            // It was silent, and that made a failing platform indistinguishable
            // from one that legitimately matched nothing: "YouTube results never
            // appear" produced no log line of any kind, so there was no way to
            // tell which. The comment directly above already said these fail often
            // (Cloudflare, expired session), which is precisely why the failure
            // needs to be visible rather than absorbed.
            .catch((err) => {
              Logger.error(`${p} channel search failed:`, err);
              return [] as ChannelItem[];
            }),
        ),
      ]);

      // A slower platform answering an earlier keystroke must not overwrite a
      // newer result set.
      if (seq !== requestSeq.current) return;

      setSearchResults(
        rankResults(
          (twitchRows ?? []).slice(0, 8).map((r) => resultToItem(r, 'twitch')),
          providerRows.flat(),
        ),
      );
      setIsSearching(false);
    }, 300);

    return () => {
      if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current);
    };
  }, [searchInput, query, wantsTwitch, searchProviders]);

  const reset = useCallback(() => {
    setSearchInput('');
    setSearchResults([]);
    setIsSearching(false);
    setHighlightIndex(0);
  }, []);

  return {
    // query state
    searchInput,
    setSearchInput,
    query,
    isSearching,
    // results
    followingItems,
    searchItems,
    visibleItems,
    followedCount: followedStreams.length,
    // keyboard navigation
    highlightIndex,
    setHighlightIndex,
    listRef,
    // actions
    refreshFollowing: loadFollowedStreams,
    reset,
  };
}
