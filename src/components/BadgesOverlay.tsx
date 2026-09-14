import { useEffect, useState, useMemo, useRef, useSyncExternalStore } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { createPortal } from 'react-dom';
import { X, ArrowUpDown, RefreshCw, Check, Trophy, Award, Search, ExternalLink, Lock } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { getAllUserBadgesWithEarned } from '../services/badgeService';
import { getProfileFromMemoryCache, getFullProfileWithFallback } from '../services/cosmeticsCache';
import { getUlidTimestamp, getFormattedCreationDate } from '../utils/ulid';
import { Tooltip } from './ui/Tooltip';
import { motion } from 'framer-motion';
import {
  getStreamNookUserNumber,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
  subscribeCosmeticsVersion,
  getCosmeticsVersion,
  getAllCosmetics,
  getOwnedCosmeticSlugs,
  getActiveCosmeticSlug,
  setActiveCosmetic,
  subscribeAtmospheresVersion,
  getAtmospheresVersion,
} from '../services/supabaseService';
import type { CosmeticCatalogEntry } from '../services/supabaseService';
import { StreamNookTierCard } from './StreamNookBadge';
import { resolveCosmeticAsset } from './cosmeticAssets';
import { listAtmospheres, getAtmosphereUnlock } from '../services/atmospheres';
import { AtmosphereBackground } from './AtmosphereBackground';
import streamNookLogo from '../assets/streamnook-logo-128.webp';
import chatterinoLogo from '../assets/chatterino-logo.svg';
import betterttvLogo from '../assets/betterttv-logo.png';
import { ChatClientsGallery } from './ChatClientsGallery';
import type { ChatClientBadge } from './ChatClientsGallery';

import { Logger } from '../utils/logger';
import { deriveBadgeStatus } from '../utils/badgeWindow';
// Tab navigation types
type AttainableTab = 'twitch-badges' | '7tv-badges' | '7tv-paints' | 'streamnook' | 'bttv' | 'chat-clients';
// Sub-tabs within the StreamNook section (its own badges vs its atmospheres).
type StreamNookTab = 'badges' | 'atmospheres';

// The Chat Clients tab (sections, tiles, provider metadata) lives in
// ChatClientsGallery.tsx; this file only fetches the badges and hands them over.

interface BadgeVersion {
  id: string;
  image_url_1x: string;
  image_url_2x: string;
  image_url_4x: string;
  title: string;
  description: string;
  click_action: string | null;
  click_url: string | null;
}

interface BadgeMetadata {
  date_added: string | null;
  usage_stats: string | null;
  more_info: string | null;
  /// Campaign-grounded writeup from the badge relay. Carries the authoritative
  /// earn window, which is what classifies a badge as live or upcoming.
  enrichment?: Record<string, unknown> | null;
  info_url: string;
}

interface BadgeWithMetadata extends BadgeVersion {
  set_id: string;
  badgebase_info?: BadgeMetadata;
}

type SortOption = 'date-newest' | 'date-oldest' | 'usage-high' | 'usage-low' | 'available' | 'coming-soon';

interface BadgeSet {
  set_id: string;
  versions: BadgeVersion[];
}

// 7TV Types
interface SevenTVImage {
  url: string;
  mime: string | null;
  scale: number | null;
  width: number | null;
  height: number | null;
  frameCount: number | null;  // camelCase from Rust serde rename
}

interface SevenTVGlobalBadge {
  id: string;
  name: string;
  description: string | null;
  tags: string[];
  images: SevenTVImage[];
  updatedAt: string | null;  // camelCase from Rust serde rename
}

interface SevenTVPaintLayer {
  id: string;
  opacity: number;
  ty: any; // Rust renames to "ty" - Complex union type from API
}

interface SevenTVPaintShadow {
  color: { hex: string; r: number; g: number; b: number; a: number };
  offsetX: number;  // camelCase from Rust serde rename
  offsetY: number;  // camelCase from Rust serde rename
  blur: number;
}

interface SevenTVGlobalPaint {
  id: string;
  name: string;
  description: string | null;
  tags: string[];
  data: {
    layers: SevenTVPaintLayer[];
    shadows: SevenTVPaintShadow[];
  } | null;
  updatedAt: string | null;  // camelCase from Rust serde rename
}

interface BadgesOverlayProps {
  onClose: () => void;
  onBadgeClick: (badge: BadgeVersion, setId: string) => void;
  initialPaintId?: string | null;
  initialBadgeId?: string | null;
  /** When true, open with the StreamNook tab active. Set by the AppStore
      action `openBadgesOnStreamNook()`. Fired by the StreamNook badge in chat
      rows + UserProfileCard. */
  initialStreamNook?: boolean;
  /** Generic deep-link: open `tab` and filter to `query` (a badge title).
      Used for tabs without a detail modal (Twitch, BetterTTV, Chat Clients)
      clicked from the profile card. */
  initialTarget?: { tab: string; query?: string } | null;
}

const BadgesOverlay = ({ onClose, onBadgeClick, initialPaintId, initialBadgeId, initialStreamNook, initialTarget }: BadgesOverlayProps) => {
  // Shallow-compared selector so unrelated store writes (toasts, viewer
  // counts) stop re-rendering the overlay.
  const { isAuthenticated, currentUser, currentStream } = useAppStore(
    useShallow((s) => ({
      isAuthenticated: s.isAuthenticated,
      currentUser: s.currentUser,
      currentStream: s.currentStream,
    })),
  );
  
  // Tab state
  const [activeTab, setActiveTab] = useState<AttainableTab>('twitch-badges');
  const [snTab, setSnTab] = useState<StreamNookTab>('badges');
  
  // Twitch badges state
  const [badges, setBadges] = useState<BadgeSet[]>([]);
  const [badgesWithMetadata, setBadgesWithMetadata] = useState<BadgeWithMetadata[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMetadata, setLoadingMetadata] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sortBy, setSortBy] = useState<SortOption>('date-newest');
  const [cacheAge, setCacheAge] = useState<number | null>(null);
  const [newBadgesCount, setNewBadgesCount] = useState(0);
  const [showRankList, setShowRankList] = useState(false);
  const [dropdownPosition, setDropdownPosition] = useState<{ top: number; left: number } | null>(null);
  const rankButtonRef = useRef<HTMLButtonElement>(null);
  
  // 7TV state
  const [seventvBadges, setSeventvBadges] = useState<SevenTVGlobalBadge[]>([]);
  const [seventvPaints, setSeventvPaints] = useState<SevenTVGlobalPaint[]>([]);
  const [loadingSeventvBadges, setLoadingSeventvBadges] = useState(false);
  const [loadingSeventvPaints, setLoadingSeventvPaints] = useState(false);
  const [seventvBadgesError, setSeventvBadgesError] = useState<string | null>(null);
  const [seventvPaintsError, setSeventvPaintsError] = useState<string | null>(null);
  
  // 7TV sort state
  const [seventvBadgeSortBy, setSeventvBadgeSortBy] = useState<'newest' | 'oldest' | 'name'>('newest');
  const [seventvPaintSortBy, setSeventvPaintSortBy] = useState<'newest' | 'oldest' | 'name' | 'most-used' | 'least-used'>('newest');

  // 7TV paint filter state
  const [seventvPaintFilter, setSeventvPaintFilter] = useState<'all' | 'animated' | 'static'>('all');

  // Global paint usage counts (paint id -> users currently wearing it). 7TV
  // exposes no usage data, so this comes from a separate stats source and only
  // covers paints it has observed; paints absent from the map have unknown usage
  // and are ordered last by the most/least-used sorts.
  const [paintUsage, setPaintUsage] = useState<Map<string, number>>(new Map());
  
  // Selected 7TV item for detail view
  const [selectedSeventvBadge, setSelectedSeventvBadge] = useState<SevenTVGlobalBadge | null>(null);
  const [selectedSeventvPaint, setSelectedSeventvPaint] = useState<SevenTVGlobalPaint | null>(null);
  
  // User's collected global badges (Set of "setId_version" keys)
  const [collectedBadgeKeys, setCollectedBadgeKeys] = useState<Set<string>>(() => {
    try {
      const user = useAppStore.getState().currentUser;
      if (user?.user_id) {
        const cached = localStorage.getItem(`streamnook_collected_badges_${user.user_id}`);
        if (cached) {
          const parsed = JSON.parse(cached);
          if (Array.isArray(parsed)) {
            Logger.debug(`[BadgesOverlay] Optimistically loaded ${parsed.length} collected badges from local storage`);
            return new Set(parsed);
          }
        }
      }
    } catch (e) {
      Logger.error('[BadgesOverlay] Failed to parse cached collected badges:', e);
    }
    return new Set();
  });
  const [loadingUserBadges, setLoadingUserBadges] = useState(false);
  
  // Search state
  const [searchQuery, setSearchQuery] = useState('');
  
  // User's owned 7TV cosmetics
  const [userOwned7TVBadgeIds, setUserOwned7TVBadgeIds] = useState<Set<string>>(new Set());
  const [userOwned7TVPaintIds, setUserOwned7TVPaintIds] = useState<Set<string>>(new Set());
  const [loadingUser7TVCosmetics, setLoadingUser7TVCosmetics] = useState(false);

  // Chat-client (third-party) badge gallery state
  const [chatClientBadges, setChatClientBadges] = useState<ChatClientBadge[]>([]);
  const [loadingChatClientBadges, setLoadingChatClientBadges] = useState(false);
  const [chatClientBadgesError, setChatClientBadgesError] = useState<string | null>(null);
  // The signed-in user's own BetterTTV Pro loyalty badge (if any), shown as an
  // "owned" tile on the BetterTTV tab. Resolved via the same on-demand socket
  // lookup the profile card uses (see bttv_pro_service.rs).
  const [myBttvProBadge, setMyBttvProBadge] = useState<{ url: string; started_at: string | null; glow: boolean } | null>(null);
  // Every distinct BTTV Pro badge image discovered across all users (persisted
  // server-side). Rendered as tiles in the BetterTTV tab regardless of ownership.
  const [discoveredProBadges, setDiscoveredProBadges] = useState<string[]>([]);

  // Load all data on mount (eager load for tab counts)
  useEffect(() => {
    loadBadges();
    // Eager load 7TV data for tab counts
    loadSeventvBadges();
    loadSeventvPaints();
    // Eager load chat-client badges for the tab count
    loadChatClientBadges();
  }, []);

  // Live-refresh the Twitch tab when the relay pushes a drop: the Rust side has
  // already merged the badge into the global cache and stored its enrichment, so
  // re-reading the cache surfaces the new tile (and its rich More Info) with no
  // manual refresh.
  useEffect(() => {
    const unlisten = listen('badge-metadata-amended', () => {
      loadBadges();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Deep-link to the StreamNook tab. Fires synchronously since the tab
  // content doesn't depend on async-loaded data.
  useEffect(() => {
    if (initialStreamNook) {
      setActiveTab('streamnook');
    }
  }, [initialStreamNook]);

  // Re-render when the StreamNook registry updates so the current user's
  // tier card surfaces inside the StreamNook tab as soon as the registry resolves.
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  const currentUserStreamNookNumber = currentUser?.user_id ? getStreamNookUserNumber(currentUser.user_id) : null;

  // Atmospheres library: the full catalog (including CS2 Major Cologne, which is
  // an atmosphere too), re-rendered when the registry loads or changes.
  useSyncExternalStore(subscribeAtmospheresVersion, getAtmospheresVersion, getAtmospheresVersion);
  const atmosphereLibrary = listAtmospheres();

  // Cosmetics catalog + ownership for the StreamNook tab grid.
  useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion, getCosmeticsVersion);
  const ownedCosmeticSlugs = currentUser?.user_id
    ? getOwnedCosmeticSlugs(currentUser.user_id)
    : new Set<string>();
  // Only wearable badges belong in this grid. Atmospheres also live in the
  // cosmetics catalog (kind 'atmosphere') for ownership, but they are applied
  // from the Atmospheres picker, not worn as the icon. Hidden cosmetics
  // (owner-only badges) are kept out of the public collection unless the viewer
  // actually owns one, so they can still equip it from here.
  const cosmeticsCatalog = getAllCosmetics().filter(
    (c) => c.kind === 'badge' && (!c.hidden || ownedCosmeticSlugs.has(c.slug)),
  );
  const activeCosmeticSlug = currentUser?.user_id ? getActiveCosmeticSlug(currentUser.user_id) : null;
  const [selectedCosmetic, setSelectedCosmetic] = useState<CosmeticCatalogEntry | null>(null);

  // Handle deep link to specific paint
  useEffect(() => {
    if (!initialPaintId || loadingSeventvPaints) return;
    
    // Switch to 7TV paints tab
    setActiveTab('7tv-paints');
    
    // Find the paint and open detail modal
    const paint = seventvPaints.find(p => p.id === initialPaintId);
    if (paint) {
      setSelectedSeventvPaint(paint);
      // Clear the initial paint ID in AppStore so it doesn't retrigger
      useAppStore.getState().setShowBadgesOverlay(true);
    }
  }, [initialPaintId, seventvPaints, loadingSeventvPaints]);

  // Handle deep link to specific 7TV badge
  useEffect(() => {
    if (!initialBadgeId || loadingSeventvBadges) return;
    
    // Switch to 7TV badges tab
    setActiveTab('7tv-badges');
    
    // Find the badge and open detail modal
    const badge = seventvBadges.find(b => b.id === initialBadgeId);
    if (badge) {
      setSelectedSeventvBadge(badge);
      // Clear the initial badge ID in AppStore so it doesn't retrigger
      useAppStore.getState().setShowBadgesOverlay(true);
    }
  }, [initialBadgeId, seventvBadges, loadingSeventvBadges]);

  // Generic deep-link: switch to the requested tab and (if given) filter to the
  // badge title so the clicked badge surfaces. Used by Twitch / BetterTTV /
  // Chat Clients tiles clicked from the profile card.
  useEffect(() => {
    if (!initialTarget) return;
    setActiveTab(initialTarget.tab as AttainableTab);
    setSearchQuery(initialTarget.query ?? '');
  }, [initialTarget]);

  // Resolve the signed-in user's own BTTV Pro badge for the BetterTTV tab.
  useEffect(() => {
    const uid = currentUser?.user_id;
    if (!uid) { setMyBttvProBadge(null); return; }
    let cancelled = false;
    invoke<{ url: string; started_at: string | null; glow: boolean } | null>('get_bttv_pro_badge', { userId: uid })
      .then((badge) => { if (!cancelled) setMyBttvProBadge(badge); })
      .catch((err) => Logger.debug('[Attainables] BTTV Pro lookup failed:', err));
    return () => { cancelled = true; };
  }, [currentUser?.user_id]);

  // Pull the growing set of discovered BTTV Pro badges. Refetched on tab change
  // so designs encountered while browsing profiles show up next time the tab opens.
  useEffect(() => {
    let cancelled = false;
    invoke<string[]>('get_discovered_bttv_pro_badges')
      .then((urls) => { if (!cancelled) setDiscoveredProBadges(urls); })
      .catch((err) => Logger.debug('[Attainables] discovered BTTV Pro fetch failed:', err));
    return () => { cancelled = true; };
  }, [activeTab]);

  // Load user's collected badges when authenticated
  useEffect(() => {
    if (isAuthenticated && currentUser) {
      loadUserBadges();
      loadUser7TVCosmetics();
    }
  }, [isAuthenticated, currentUser]);

  // Load user's badges using unified badge service
  const loadUserBadges = async () => {
    if (!currentUser) return;
    
    setLoadingUserBadges(true);
    try {
      const channelId = currentStream?.user_id || currentUser.user_id;
      const channelName = currentStream?.user_login || currentUser.login || currentUser.username;
      
      // Use unified badge service with full earned badge collection
      const badgeData = await getAllUserBadgesWithEarned(
        currentUser.user_id,
        currentUser.login || currentUser.username,
        channelId,
        channelName
      );
      
      // Create a Set of badge keys the user owns (display badges + earned badges)
      const keys = new Set<string>();
      
      // Add display badges
      badgeData.displayBadges?.forEach((badge: any) => {
        if (badge && badge.setID && badge.version) {
          keys.add(`${badge.setID}_${badge.version}`);
        }
      });
      
      // Add earned badges
      badgeData.earnedBadges?.forEach((badge: any) => {
        if (badge && badge.setID && badge.version) {
          keys.add(`${badge.setID}_${badge.version}`);
        }
      });
      
      // An empty result is far more likely a failed lookup (Twitch GQL drift,
      // no Drops token yet, offline) than a user with zero badges, and the
      // Rust side reports those failures as an empty list. Keep the last good
      // set on screen and in localStorage rather than wiping it.
      if (keys.size === 0) {
        Logger.warn('[BadgesOverlay] Badge lookup returned no badges; keeping the cached collection');
        return;
      }

      setCollectedBadgeKeys(keys);
      try {
        localStorage.setItem(
          `streamnook_collected_badges_${currentUser.user_id}`,
          JSON.stringify(Array.from(keys))
        );
      } catch (e) {
        Logger.error('[BadgesOverlay] Failed to cache collected badges:', e);
      }
      Logger.debug(`[BadgesOverlay] User has ${keys.size} collected badges`);
    } catch (err) {
      Logger.error('[BadgesOverlay] Failed to load user badges:', err);
    } finally {
      setLoadingUserBadges(false);
    }
  };

  // Load user's 7TV cosmetics for collection counters
  const loadUser7TVCosmetics = async () => {
    if (!currentUser) return;
    
    setLoadingUser7TVCosmetics(true);
    try {
      const channelId = currentStream?.user_id || currentUser.user_id;
      const channelName = currentStream?.user_login || currentUser.login || currentUser.username;
      
      // Try memory cache first
      let profile = getProfileFromMemoryCache(currentUser.user_id);
      
      // If no cache, fetch from API
      if (!profile) {
        profile = await getFullProfileWithFallback(
          currentUser.user_id,
          currentUser.login || currentUser.username,
          channelId,
          channelName
        );
      }
      
      // Extract owned badge and paint IDs
      const badgeIds = new Set<string>();
      const paintIds = new Set<string>();
      
      profile.seventvCosmetics.badges?.forEach((badge: any) => {
        if (badge?.id) badgeIds.add(badge.id);
      });
      
      profile.seventvCosmetics.paints?.forEach((paint: any) => {
        if (paint?.id) paintIds.add(paint.id);
      });
      
      setUserOwned7TVBadgeIds(badgeIds);
      setUserOwned7TVPaintIds(paintIds);
      Logger.debug(`[BadgesOverlay] User owns ${badgeIds.size} 7TV badges and ${paintIds.size} 7TV paints`);
    } catch (err) {
      Logger.error('[BadgesOverlay] Failed to load user 7TV cosmetics:', err);
    } finally {
      setLoadingUser7TVCosmetics(false);
    }
  };

  // Load 7TV badges when tab is activated
  const loadSeventvBadges = async () => {
    if (seventvBadges.length > 0 || loadingSeventvBadges) return; // Already loaded or loading
    
    setLoadingSeventvBadges(true);
    setSeventvBadgesError(null);
    try {
      Logger.debug('[Attainables] Fetching 7TV badges...');
      const badges = await invoke<SevenTVGlobalBadge[]>('get_all_seventv_badges');
      setSeventvBadges(badges);
      Logger.debug(`[Attainables] Loaded ${badges.length} 7TV badges`);
    } catch (err) {
      Logger.error('[Attainables] Failed to load 7TV badges:', err);
      setSeventvBadgesError('Failed to load 7TV badges');
    } finally {
      setLoadingSeventvBadges(false);
    }
  };

  // Load 7TV paints when tab is activated
  const loadSeventvPaints = async () => {
    if (seventvPaints.length > 0 || loadingSeventvPaints) return; // Already loaded or loading
    
    setLoadingSeventvPaints(true);
    setSeventvPaintsError(null);
    try {
      Logger.debug('[Attainables] Fetching 7TV paints...');
      const paints = await invoke<SevenTVGlobalPaint[]>('get_all_seventv_paints');
      setSeventvPaints(paints);
      Logger.debug(`[Attainables] Loaded ${paints.length} 7TV paints`);
    } catch (err) {
      Logger.error('[Attainables] Failed to load 7TV paints:', err);
      setSeventvPaintsError('Failed to load 7TV paints');
    } finally {
      setLoadingSeventvPaints(false);
    }
  };

  // Load global paint usage counts so the paints tab can sort by most/least
  // used. Best-effort: if it fails the tab still works, the usage sorts just
  // fall back to ordering everything as unknown.
  const loadSeventvPaintUsage = async () => {
    if (paintUsage.size > 0) return; // Already loaded
    try {
      const usage = await invoke<{ id: string; user_count: number }[]>('get_seventv_paint_usage');
      const map = new Map<string, number>();
      usage.forEach(u => map.set(u.id, u.user_count));
      setPaintUsage(map);
      Logger.debug(`[Attainables] Loaded usage counts for ${map.size} paints`);
    } catch (err) {
      Logger.error('[Attainables] Failed to load paint usage counts:', err);
    }
  };

  // Load chat-client (third-party) badges when tab is activated
  const loadChatClientBadges = async () => {
    if (chatClientBadges.length > 0 || loadingChatClientBadges) return;

    setLoadingChatClientBadges(true);
    setChatClientBadgesError(null);
    try {
      Logger.debug('[Attainables] Fetching chat-client badges...');
      const badges = await invoke<ChatClientBadge[]>('get_all_third_party_badges', {
        viewerUserId: currentUser?.user_id ?? null,
      });
      setChatClientBadges(badges);
      Logger.debug(`[Attainables] Loaded ${badges.length} chat-client badges`);
    } catch (err) {
      Logger.error('[Attainables] Failed to load chat-client badges:', err);
      setChatClientBadgesError('Failed to load chat-client badges');
    } finally {
      setLoadingChatClientBadges(false);
    }
  };

  // Load data when tab changes
  useEffect(() => {
    if (activeTab === '7tv-badges') {
      loadSeventvBadges();
    } else if (activeTab === '7tv-paints') {
      loadSeventvPaints();
      loadSeventvPaintUsage();
    } else if (activeTab === 'chat-clients') {
      loadChatClientBadges();
    }
  }, [activeTab]);

  // Check if a 7TV badge/image is animated (frameCount > 1)
  const isAnimatedBadge = (badge: SevenTVGlobalBadge): boolean => {
    return badge.images?.some(img => (img.frameCount || 0) > 1) || false;
  };

  // Get best image URL for 7TV badge (prefer animated webp if available)
  const getSeventvBadgeImageUrl = (badge: SevenTVGlobalBadge, preferAnimated = true): string => {
    if (!badge.images || badge.images.length === 0) return '';
    
    // Prefer webp format (supports animation)
    const webpImages = badge.images.filter(img => img.mime === 'image/webp');
    
    // Check for animated versions first (frameCount > 1)
    if (preferAnimated) {
      const animatedImages = webpImages.filter(img => (img.frameCount || 0) > 1);
      if (animatedImages.length > 0) {
        // Get highest scale animated image
        const scale4 = animatedImages.find(img => img.scale === 4);
        const scale3 = animatedImages.find(img => img.scale === 3);
        const scale2 = animatedImages.find(img => img.scale === 2);
        if (scale4?.url) return scale4.url;
        if (scale3?.url) return scale3.url;
        if (scale2?.url) return scale2.url;
        return animatedImages[0]?.url || '';
      }
    }
    
    // Fall back to static highest scale
    const scale4 = webpImages.find(img => img.scale === 4);
    const scale2 = webpImages.find(img => img.scale === 2);
    const scale1 = webpImages.find(img => img.scale === 1);
    
    return scale4?.url || scale2?.url || scale1?.url || badge.images[0]?.url || '';
  };

  // Check if a 7TV paint is animated (has image layer with frameCount > 1)
  const isAnimatedPaint = (paint: SevenTVGlobalPaint): boolean => {
    if (!paint.data?.layers?.[0]?.ty) return false;
    const layerType = paint.data.layers[0].ty;
    if (layerType.__typename !== 'PaintLayerTypeImage' || !layerType.images) return false;
    return layerType.images.some((img: any) => (img.frameCount || 0) > 1);
  };

  // Get animated paint image URL (prefer animated webp)
  // preferScale: lower = less decode overhead. Use 1 for grid thumbnails, 2-3 for detail modal.
  const getAnimatedPaintImageUrl = (paint: SevenTVGlobalPaint, preferScale: 1 | 2 | 3 | 4 = 4): string | null => {
    if (!paint.data?.layers?.[0]?.ty) return null;
    const layerType = paint.data.layers[0].ty;
    if (layerType.__typename !== 'PaintLayerTypeImage' || !layerType.images) return null;
    
    // Find animated webp images (frameCount > 1, not containing '_static')
    const animatedImages = layerType.images.filter((img: any) => 
      img.mime === 'image/webp' && 
      (img.frameCount || 0) > 1 && 
      !img.url.includes('_static')
    );
    
    if (animatedImages.length === 0) return null;
    
    // Select by preferred scale (lower = less decode overhead for small cards)
    for (let s = preferScale; s >= 1; s--) {
      const match = animatedImages.find((img: any) => img.scale === s);
      if (match?.url) return match.url;
    }
    // Fallback: try scales above preferred
    for (let s = preferScale + 1; s <= 4; s++) {
      const match = animatedImages.find((img: any) => img.scale === s);
      if (match?.url) return match.url;
    }
    return animatedImages[0]?.url || null;
  };

  // Generate CSS gradient from 7TV paint layers
  // preferScale: controls animated image resolution. Use 1 for grid, 2 for detail modal.
  const generatePaintGradient = (paint: SevenTVGlobalPaint, preferScale: 1 | 2 | 3 | 4 = 4): string => {
    if (!paint.data || !paint.data.layers || paint.data.layers.length === 0) {
      return 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)'; // Default fallback
    }

    const layer = paint.data.layers[0];
    const layerType = layer.ty;
    
    if (!layerType) return 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)';

    // Handle different layer types
    if (layerType.__typename === 'PaintLayerTypeSingleColor' && layerType.color) {
      return layerType.color.hex;
    }

    if (layerType.__typename === 'PaintLayerTypeLinearGradient' && layerType.stops) {
      const angle = layerType.angle || 90;
      const stops = layerType.stops
        .map((stop: any) => `${stop.color.hex} ${Math.round(stop.at * 100)}%`)
        .join(', ');
      return `linear-gradient(${angle}deg, ${stops})`;
    }

    if (layerType.__typename === 'PaintLayerTypeRadialGradient' && layerType.stops) {
      const stops = layerType.stops
        .map((stop: any) => `${stop.color.hex} ${Math.round(stop.at * 100)}%`)
        .join(', ');
      return `radial-gradient(circle, ${stops})`;
    }

    if (layerType.__typename === 'PaintLayerTypeImage' && layerType.images?.[0]?.url) {
      // For image paints, prefer animated webp if available
      const animatedUrl = getAnimatedPaintImageUrl(paint, preferScale);
      return `url(${animatedUrl || layerType.images[0].url})`;
    }

    return 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)';
  };

  // Build a CSS drop-shadow filter from a 7TV paint's shadows. Mirrors
  // computeDropShadows in seventvService.ts (one drop-shadow() per layer, blur
  // 1:1) so the overlay renders shadows identically to chat/profile. Returns a
  // ready-to-use filter value, or 'none' when the paint has no shadows.
  const generatePaintShadow = (paint: SevenTVGlobalPaint): string => {
    if (!paint.data?.shadows || paint.data.shadows.length === 0) {
      return 'none';
    }

    return paint.data.shadows
      .map(shadow => {
        const offsetX = shadow.offsetX || 0;
        const offsetY = shadow.offsetY || 0;
        const blur = shadow.blur || 0;
        const color = shadow.color?.hex || '#000000';
        return `drop-shadow(${color} ${offsetX}px ${offsetY}px ${blur}px)`;
      })
      .join(' ');
  };

  // Check if user has collected a specific badge
  const isCollected = (badge: BadgeWithMetadata): boolean => {
    return collectedBadgeKeys.has(`${badge.set_id}_${badge.id}`);
  };

  // Sort 7TV badges based on current sort option
  const sortedSeventvBadges = useMemo(() => {
    const filtered = seventvBadges.filter(badge => 
      !searchQuery || badge.name.toLowerCase().includes(searchQuery.toLowerCase())
    );
    
    return [...filtered].sort((a, b) => {
      switch (seventvBadgeSortBy) {
        case 'newest':
          return getUlidTimestamp(b.id) - getUlidTimestamp(a.id);
        case 'oldest':
          return getUlidTimestamp(a.id) - getUlidTimestamp(b.id);
        case 'name':
          return a.name.localeCompare(b.name);
        default:
          return 0;
      }
    });
  }, [seventvBadges, seventvBadgeSortBy, searchQuery]);

  // Sort 7TV paints based on current sort option
  const sortedSeventvPaints = useMemo(() => {
    let filtered = seventvPaints.filter(paint => 
      !searchQuery || paint.name.toLowerCase().includes(searchQuery.toLowerCase())
    );
    
    // Apply animation filter
    if (seventvPaintFilter === 'animated') {
      filtered = filtered.filter(paint => isAnimatedPaint(paint));
    } else if (seventvPaintFilter === 'static') {
      filtered = filtered.filter(paint => !isAnimatedPaint(paint));
    }
    
    return [...filtered].sort((a, b) => {
      switch (seventvPaintSortBy) {
        case 'newest':
          return getUlidTimestamp(b.id) - getUlidTimestamp(a.id);
        case 'oldest':
          return getUlidTimestamp(a.id) - getUlidTimestamp(b.id);
        case 'name':
          return a.name.localeCompare(b.name);
        case 'most-used':
        case 'least-used': {
          // Usage data only covers a subset of paints; the rest have unknown
          // usage and always trail the ranked ones (so "least used" surfaces the
          // genuinely least-worn tracked paints, not a wall of unknowns).
          const aHas = paintUsage.has(a.id);
          const bHas = paintUsage.has(b.id);
          if (aHas !== bHas) return aHas ? -1 : 1;
          if (!aHas) return getUlidTimestamp(b.id) - getUlidTimestamp(a.id);
          const aUsage = paintUsage.get(a.id)!;
          const bUsage = paintUsage.get(b.id)!;
          if (aUsage !== bUsage) {
            return seventvPaintSortBy === 'most-used' ? bUsage - aUsage : aUsage - bUsage;
          }
          return a.name.localeCompare(b.name);
        }
        default:
          return 0;
      }
    });
  }, [seventvPaints, seventvPaintSortBy, seventvPaintFilter, searchQuery, paintUsage]);

  // Badge set IDs that are NOT true global collectibles and shouldn't count towards collection
  // These badges are either: channel-specific, role-based, paid-only, or not earnable by regular users
  const channelSpecificBadgeSets = new Set([
    // Channel-specific badges
    'subscriber',          // Channel subscriptions (1-month, 2-month, 3-month, 6-month, etc.)
    'sub-gifter',          // Gift sub badges (varies by count)
    'sub-gift-leader',     // Sub gift leaderboard
    'founder',             // Channel founder
    'vip',                 // Channel VIP
    'moderator',           // Channel moderator
    'artist-badge',        // Channel artist
    'moments',             // Channel moments
    
    // Cheering / Bits badges
    'bits',                // Bits cheering badges (Cheer 1, 100, 1000, 5000, 10000, 100000)
    'bits-leader',         // Bits leaderboard
    'bits-charity',        // Bits for charity
    'anonymous-cheerer',   // Anonymous cheering
    
    // Hype Train
    'hype-train',          // Hype train conductors
    
    // Predictions
    'predictions',         // Predicted badges (blue/pink)
    
    // GIF-related badges
    'sub-gift-count',      // GIF subs
    'clip-champ',          // Clips Leader
    'clips-leader',        // Clips Leader alternate
    'gift-leader',         // GIF Leader / GIFter Leader
    'gifter-leader',       // GIFter Leader alternate
    
    // Twitch Staff & Special Roles (not earnable by regular users)
    'staff',               // Twitch staff
    'admin',               // Twitch admin
    'global_mod',          // Global mod
    'broadcaster',         // Broadcaster badge
    'verified-moderator',  // Verified Moderator
    'automod',             // AutoMod
    'chatbot',             // ChatBot badge
    'twitch-intern',       // Twitch Intern badges
    'lead-moderator',      // Lead Moderator
    
    // Paid / Subscription-based badges (not globally earnable)
    'turbo',               // Twitch Turbo
    'prime',               // Prime Gaming
    'prime-gaming',        // Prime Gaming alternate
    
    // Ambassador / Partner program badges
    'ambassador',          // Twitch Ambassador
    'partner',             // Twitch Partner
    
    // Anniversary badges
    'twitchanniversary',   // Twitch Anniversary
    'twitch-anniversary',  // Twitch Anniversary alternate
    
    // Developer badges
    'game-developer',      // Game Developer badge
    'extension',           // Extension developer
    
    // Accessibility badges (not collectibles)
    'no_audio',            // Watching without audio
    'no_video',            // Listening only
    
    // Event-specific / Limited badges that aren't collectible
    'survival-cup-4',      // Survival Cup 4
  ]);

  // Check if a badge is a true "global" collectible badge
  const isGlobalCollectibleBadge = (badge: BadgeWithMetadata): boolean => {
    return !channelSpecificBadgeSets.has(badge.set_id);
  };

  // Filter badges to only include true global collectibles
  const globalCollectibleBadges = useMemo(() => {
    return badgesWithMetadata.filter(isGlobalCollectibleBadge);
  }, [badgesWithMetadata]);

  // Count collected global badges
  const collectedCount = useMemo(() => {
    if (collectedBadgeKeys.size === 0) return 0;
    return globalCollectibleBadges.filter(badge => isCollected(badge)).length;
  }, [globalCollectibleBadges, collectedBadgeKeys]);

  // Total global collectible badges
  const totalGlobalBadges = globalCollectibleBadges.length;

  // Collection rank system based on percentage collected - Epic tier system
  const getCollectionRank = (collected: number, total: number) => {
    if (total === 0) return null;
    const percentage = (collected / total) * 100;
    
    // 10 Epic rank tiers with unique themes
    if (percentage >= 95) {
      return {
        title: 'APEX',
        tier: 'apex',
        description: 'The final form',
        animationClass: 'rank-apex',
        colors: {
          from: '#ff0080',
          via: '#7928ca',
          to: '#00d4ff',
          glow: 'rgba(121, 40, 202, 0.5)',
          bg: 'from-[#ff0080]/20 via-[#7928ca]/20 to-[#00d4ff]/20',
          border: '[#7928ca]/50',
          sparkle: ['#ff0080', '#7928ca', '#00d4ff', '#ff6b6b', '#feca57']
        }
      };
    } else if (percentage >= 85) {
      return {
        title: 'TITAN',
        tier: 'titan',
        description: 'Diamond incarnate',
        animationClass: 'rank-titan',
        colors: {
          from: '#e8e8e8',
          via: '#c0c0c0',
          to: '#a8d8ea',
          glow: 'rgba(200, 200, 220, 0.5)',
          bg: 'from-[#e8e8e8]/15 via-[#c0c0c0]/15 to-[#a8d8ea]/15',
          border: '[#c0c0c0]/40',
          sparkle: ['#ffffff', '#e8e8e8', '#a8d8ea', '#ffd700']
        }
      };
    } else if (percentage >= 73) {
      return {
        title: 'AEON',
        tier: 'aeon',
        description: 'Cosmic wanderer',
        animationClass: 'rank-aeon',
        colors: {
          from: '#1a1a2e',
          via: '#4a0080',
          to: '#ffd700',
          glow: 'rgba(74, 0, 128, 0.4)',
          bg: 'from-[#1a1a2e]/20 via-[#4a0080]/20 to-[#ffd700]/10',
          border: '[#ffd700]/30',
          sparkle: ['#ffd700', '#4a0080', '#ffffff', '#ff6b6b']
        }
      };
    } else if (percentage >= 59) {
      return {
        title: 'NEXUS',
        tier: 'nexus',
        description: 'Grid architect',
        animationClass: 'rank-nexus',
        colors: {
          from: '#7c3aed',
          via: '#a855f7',
          to: '#c084fc',
          glow: 'rgba(124, 58, 237, 0.4)',
          bg: 'from-[#7c3aed]/15 via-[#a855f7]/15 to-[#c084fc]/15',
          border: '[#7c3aed]/40',
          sparkle: ['#7c3aed', '#a855f7', '#c084fc', '#e879f9']
        }
      };
    } else if (percentage >= 47) {
      return {
        title: 'AURORA',
        tier: 'aurora',
        description: 'Northern light bearer',
        animationClass: 'rank-aurora',
        colors: {
          from: '#14b8a6',
          via: '#a855f7',
          to: '#ec4899',
          glow: 'rgba(20, 184, 166, 0.35)',
          bg: 'from-[#14b8a6]/12 via-[#a855f7]/12 to-[#ec4899]/12',
          border: '[#14b8a6]/35',
          sparkle: ['#14b8a6', '#a855f7', '#ec4899', '#06b6d4']
        }
      };
    } else if (percentage >= 35) {
      return {
        title: 'VANGUARD',
        tier: 'vanguard',
        description: 'Chrome sentinel',
        animationClass: 'rank-vanguard',
        colors: {
          from: '#94a3b8',
          via: '#64748b',
          to: '#cbd5e1',
          glow: 'rgba(148, 163, 184, 0.35)',
          bg: 'from-[#94a3b8]/12 via-[#64748b]/12 to-[#cbd5e1]/12',
          border: '[#94a3b8]/35',
          sparkle: ['#94a3b8', '#cbd5e1', '#e2e8f0', '#f1f5f9']
        }
      };
    } else if (percentage >= 23) {
      return {
        title: 'PHANTOM',
        tier: 'phantom',
        description: 'Ethereal presence',
        animationClass: 'rank-phantom',
        colors: {
          from: '#06b6d4',
          via: '#22d3d1',
          to: '#67e8f9',
          glow: 'rgba(6, 182, 212, 0.35)',
          bg: 'from-[#06b6d4]/12 via-[#22d3d1]/12 to-[#67e8f9]/12',
          border: '[#06b6d4]/35',
          sparkle: ['#06b6d4', '#22d3d1', '#67e8f9', '#a5f3fc']
        }
      };
    } else if (percentage >= 13) {
      return {
        title: 'RONIN',
        tier: 'ronin',
        description: 'Blade of the void',
        animationClass: 'rank-ronin',
        colors: {
          from: '#3b82f6',
          via: '#60a5fa',
          to: '#0ea5e9',
          glow: 'rgba(59, 130, 246, 0.4)',
          bg: 'from-[#3b82f6]/15 via-[#60a5fa]/15 to-[#0ea5e9]/15',
          border: '[#3b82f6]/40',
          sparkle: ['#3b82f6', '#60a5fa', '#0ea5e9', '#38bdf8']
        }
      };
    } else if (percentage >= 6) {
      return {
        title: 'NOMAD',
        tier: 'nomad',
        description: 'Desert wanderer',
        animationClass: 'rank-nomad',
        colors: {
          from: '#78716c',
          via: '#a8a29e',
          to: '#d4a84b',
          glow: 'rgba(212, 168, 75, 0.25)',
          bg: 'from-[#78716c]/10 via-[#a8a29e]/10 to-[#d4a84b]/10',
          border: '[#d4a84b]/25',
          sparkle: ['#78716c', '#a8a29e', '#d4a84b', '#f5d0a9']
        }
      };
    } else if (percentage >= 0.1) {
      return {
        title: 'DRIFTER',
        tier: 'drifter',
        description: 'Signal in the static',
        animationClass: 'rank-drifter',
        colors: {
          from: '#6b7280',
          via: '#9ca3af',
          to: '#e5e7eb',
          glow: 'rgba(156, 163, 175, 0.2)',
          bg: 'from-[#6b7280]/8 via-[#9ca3af]/8 to-[#e5e7eb]/8',
          border: '[#9ca3af]/20',
          sparkle: ['#6b7280', '#9ca3af', '#e5e7eb', '#f3f4f6']
        }
      };
    }
    return null;
  };

  // Get current rank
  const currentRank = useMemo(() => {
    return getCollectionRank(collectedCount, totalGlobalBadges);
  }, [collectedCount, totalGlobalBadges]);

  // All ranks for display in ranks list - Epic 10-tier system
  const allRanks = [
    {
      title: 'APEX',
      requirement: '95%+',
      description: 'The final form',
      tier: 'apex',
      colors: { from: '#ff0080', via: '#7928ca', to: '#00d4ff' }
    },
    {
      title: 'TITAN',
      requirement: '85%+',
      description: 'Diamond incarnate',
      tier: 'titan',
      colors: { from: '#e8e8e8', via: '#c0c0c0', to: '#a8d8ea' }
    },
    {
      title: 'AEON',
      requirement: '73%+',
      description: 'Cosmic wanderer',
      tier: 'aeon',
      colors: { from: '#1a1a2e', via: '#4a0080', to: '#ffd700' }
    },
    {
      title: 'NEXUS',
      requirement: '59%+',
      description: 'Grid architect',
      tier: 'nexus',
      colors: { from: '#7c3aed', via: '#a855f7', to: '#c084fc' }
    },
    {
      title: 'AURORA',
      requirement: '47%+',
      description: 'Northern light bearer',
      tier: 'aurora',
      colors: { from: '#14b8a6', via: '#a855f7', to: '#ec4899' }
    },
    {
      title: 'VANGUARD',
      requirement: '35%+',
      description: 'Chrome sentinel',
      tier: 'vanguard',
      colors: { from: '#94a3b8', via: '#64748b', to: '#cbd5e1' }
    },
    {
      title: 'PHANTOM',
      requirement: '23%+',
      description: 'Ethereal presence',
      tier: 'phantom',
      colors: { from: '#06b6d4', via: '#22d3d1', to: '#67e8f9' }
    },
    {
      title: 'RONIN',
      requirement: '13%+',
      description: 'Blade of the void',
      tier: 'ronin',
      colors: { from: '#3b82f6', via: '#60a5fa', to: '#0ea5e9' }
    },
    {
      title: 'NOMAD',
      requirement: '6%+',
      description: 'Desert wanderer',
      tier: 'nomad',
      colors: { from: '#78716c', via: '#a8a29e', to: '#d4a84b' }
    },
    {
      title: 'DRIFTER',
      requirement: '0.1%+',
      description: 'Signal in the static',
      tier: 'drifter',
      colors: { from: '#6b7280', via: '#9ca3af', to: '#e5e7eb' }
    }
  ];

  const loadBadges = async () => {
    try {
      setLoading(true);
      setError(null);

      // Try to load from cache first
      Logger.debug('[BadgesOverlay] Checking for cached badges...');
      const cachedBadges = await invoke<{ data: BadgeSet[] } | null>('get_cached_global_badges');

      // Also check cache age
      const age = await invoke<number | null>('get_badge_cache_age');
      setCacheAge(age);

      if (cachedBadges && cachedBadges.data && cachedBadges.data.length > 0) {
        Logger.debug('[BadgesOverlay] Found cached badges, loading immediately');
        setBadges(cachedBadges.data);

        // Flatten all badge versions
        const flattened = cachedBadges.data.flatMap(set =>
          set.versions.map(version => ({ ...version, set_id: set.set_id } as BadgeWithMetadata))
        );

        // Pre-load ALL badge metadata from cache in ONE call (fast batch lookup)
        let badgesWithPreloadedMetadata: BadgeWithMetadata[] = flattened;
        try {
          const allBadgeCache = await invoke<Record<string, { data: any; position?: number }>>('get_all_universal_cached_items', {
            cacheType: 'badge',
          });

          if (allBadgeCache && Object.keys(allBadgeCache).length > 0) {
            Logger.debug(`[BadgesOverlay] Loaded ${Object.keys(allBadgeCache).length} cached badge entries in single call`);
            badgesWithPreloadedMetadata = flattened.map(badge => {
              const cacheKey = `metadata:${badge.set_id}-v${badge.id}`;
              const cached = allBadgeCache[cacheKey];
              if (cached) {
                return {
                  ...badge,
                  badgebase_info: {
                    ...cached.data,
                    position: cached.position
                  }
                };
              }
              return badge;
            });
          }
        } catch (err) {
          Logger.error('[BadgesOverlay] Failed to batch load cache:', err);
        }

        setBadgesWithMetadata(badgesWithPreloadedMetadata);
        setLoading(false);

        // Fetch any missing metadata in the background
        fetchAllBadgeMetadata(badgesWithPreloadedMetadata);

        // Check for badges missing metadata (new badges that need BadgeBase data)
        checkAndFetchMissingMetadata();

        return;
      }

      // No cache available, fetch from API
      Logger.debug('[BadgesOverlay] No cached badges, fetching from API...');

      // Fetch global badges (Rust attaches its own credentials and caches them)
      const response = await invoke<{ data: BadgeSet[] }>('fetch_global_badges');

      setBadges(response.data);

      // Flatten all badge versions
      const flattened = response.data.flatMap(set =>
        set.versions.map(version => ({ ...version, set_id: set.set_id } as BadgeWithMetadata))
      );

      setBadgesWithMetadata(flattened);

      // Fetch metadata for all badges in the background
      fetchAllBadgeMetadata(flattened);
    } catch (err) {
      Logger.error('Failed to load badges:', err);
      setError('Failed to load badges. Please try again.');
    } finally {
      setLoading(false);
    }
  };

  // Check for badges that don't have metadata and fetch from BadgeBase
  const checkAndFetchMissingMetadata = async () => {
    try {
      Logger.debug('[BadgesOverlay] Checking for badges missing metadata...');
      const missing = await invoke<[string, string][]>('get_badges_missing_metadata');

      if (missing.length > 0) {
        Logger.debug(`[BadgesOverlay] Found ${missing.length} badges missing metadata, fetching...`);
        setNewBadgesCount(missing.length);

        // Fetch metadata for missing badges in batches
        const batchSize = 5;
        for (let i = 0; i < missing.length; i += batchSize) {
          const batch = missing.slice(i, i + batchSize);

          await Promise.allSettled(
            batch.map(([setId, version]) =>
              invoke<BadgeMetadata>('fetch_badge_metadata', {
                badgeSetId: setId,
                badgeVersion: version,
              })
            )
          );

          // Update progress
          setNewBadgesCount(Math.max(0, missing.length - (i + batchSize)));
        }

        Logger.debug('[BadgesOverlay] Finished fetching missing badge metadata');
        setNewBadgesCount(0);

        // Reload metadata to update display
        if (badgesWithMetadata.length > 0) {
          fetchAllBadgeMetadata(badgesWithMetadata);
        }
      }
    } catch (err) {
      Logger.error('[BadgesOverlay] Error checking for missing metadata:', err);
    }
  };

  // Force refresh badges from Twitch API (bypasses cache)
  const forceRefreshBadges = async () => {
    try {
      setRefreshing(true);
      Logger.debug('[BadgesOverlay] Force refreshing badges from Twitch API...');

      const response = await invoke<{ data: BadgeSet[] }>('force_refresh_global_badges');

      Logger.debug(`[BadgesOverlay] Refreshed ${response.data.length} badge sets from Twitch API`);

      // Log all badge set IDs for debugging
      const badgeSetIds = response.data.map(s => s.set_id);
      Logger.debug('[BadgesOverlay] Badge set IDs received:', badgeSetIds);

      // Count total versions
      const totalVersions = response.data.reduce((acc, set) => acc + set.versions.length, 0);
      Logger.debug(`[BadgesOverlay] Total badge versions: ${totalVersions}`);

      // Log each badge set with its versions
      response.data.forEach(set => {
        Logger.debug(`[BadgesOverlay] Set "${set.set_id}": ${set.versions.length} versions - ${set.versions.map(v => v.title).join(', ')}`);
      });

      setBadges(response.data);
      setCacheAge(0);

      // Flatten all badge versions
      const flattened = response.data.flatMap(set =>
        set.versions.map(version => ({ ...version, set_id: set.set_id } as BadgeWithMetadata))
      );

      Logger.debug(`[BadgesOverlay] Flattened to ${flattened.length} badge items`);

      setBadgesWithMetadata(flattened);

      // Fetch metadata for all badges with force=true to bypass cache
      await fetchAllBadgeMetadata(flattened, true);

      // Check for and fetch any new badges that don't have metadata yet
      await checkAndFetchMissingMetadata();

    } catch (err) {
      Logger.error('Failed to refresh badges:', err);
      setError('Failed to refresh badges. Please try again.');
    } finally {
      setRefreshing(false);
    }
  };

  const fetchAllBadgeMetadata = async (badgeList: BadgeWithMetadata[], forceRefresh: boolean = false) => {
    setLoadingMetadata(true);

    // First, load ALL badge cache in ONE call (fast batch lookup)
    const metadataCache: Record<string, BadgeMetadata> = {};
    let uncachedBadges: BadgeWithMetadata[] = [];

    // If force refresh, skip cache and fetch all badges fresh
    if (forceRefresh) {
      Logger.debug('[BadgesOverlay] Force refresh requested, fetching ALL badge metadata from BadgeBase...');
      uncachedBadges = [...badgeList];
    } else {
      Logger.debug('[BadgesOverlay] Batch loading all badge cache...');
      try {
        const allBadgeCache = await invoke<Record<string, { data: any; position?: number }>>('get_all_universal_cached_items', {
          cacheType: 'badge',
        });

        // Map badges to their cache entries
        for (const badge of badgeList) {
          const cacheKey = `metadata:${badge.set_id}-v${badge.id}`;
          const cached = allBadgeCache[cacheKey];

          if (cached) {
            const metadata = cached.data as BadgeMetadata;
            (metadata as any).position = cached.position;
            metadataCache[`${badge.set_id}/${badge.id}`] = metadata;
          } else {
            uncachedBadges.push(badge);
          }
        }

        Logger.debug(`[BadgesOverlay] Found ${Object.keys(metadataCache).length} badges in cache (batch), need to fetch ${uncachedBadges.length} from API`);
      } catch (err) {
        Logger.error('[BadgesOverlay] Failed to batch load cache, falling back to uncached:', err);
        // If batch load fails, treat all as uncached
        uncachedBadges.push(...badgeList);
      }

      // Update UI with cached data immediately
      if (Object.keys(metadataCache).length > 0) {
        const updatedBadges = badgeList.map(badge => ({
          ...badge,
          badgebase_info: metadataCache[`${badge.set_id}/${badge.id}`]
        }));
        setBadgesWithMetadata(updatedBadges);
      }
    }

    // Now fetch badges from API (all badges if force refresh, or only uncached badges)
    if (uncachedBadges.length > 0) {
      const batchSize = 10; // Process 10 badges at a time

      for (let i = 0; i < uncachedBadges.length; i += batchSize) {
        const batch = uncachedBadges.slice(i, i + batchSize);

        const batchResults = await Promise.allSettled(
          batch.map(badge =>
            invoke<BadgeMetadata>('fetch_badge_metadata', {
              badgeSetId: badge.set_id,
              badgeVersion: badge.id,
              force: forceRefresh,
            })
          )
        );

        // Process batch results
        batch.forEach((badge, index) => {
          const result = batchResults[index];
          if (result.status === 'fulfilled') {
            metadataCache[`${badge.set_id}/${badge.id}`] = result.value;
          }
        });

        // Update UI after each batch
        const updatedBadges = badgeList.map(badge => ({
          ...badge,
          badgebase_info: metadataCache[`${badge.set_id}/${badge.id}`]
        }));
        setBadgesWithMetadata(updatedBadges);
      }
    }

    setLoadingMetadata(false);
  };

  // Parse usage stats to get numeric value for sorting
  const parseUsageStats = (stats: string | null | undefined): number => {
    if (!stats) return 0;

    // Extract number from strings like "1,234 users seen with this badge" or "None users"
    const match = stats.match(/(\d+(?:,\d+)*)/);
    if (match) {
      return parseInt(match[1].replace(/,/g, ''), 10);
    }
    return 0;
  };

  // Parse date for sorting - handles multiple formats
  const parseDate = (dateStr: string | null | undefined): number => {
    if (!dateStr) return 0;
    try {
      // Month name mappings (full and abbreviated)
      const months: Record<string, number> = {
        'January': 0, 'February': 1, 'March': 2, 'April': 3,
        'May': 4, 'June': 5, 'July': 6, 'August': 7,
        'September': 8, 'October': 9, 'November': 10, 'December': 11,
        'Jan': 0, 'Feb': 1, 'Mar': 2, 'Apr': 3,
        'Jun': 5, 'Jul': 6, 'Aug': 7,
        'Sep': 8, 'Oct': 9, 'Nov': 10, 'Dec': 11
      };

      // Try to match "DD Month YYYY" format (e.g., "12 November 2025")
      const fullMatch = dateStr.match(/(\d{1,2})\s+(\w+)\s+(\d{4})/);
      if (fullMatch) {
        const day = parseInt(fullMatch[1], 10);
        const monthName = fullMatch[2];
        const year = parseInt(fullMatch[3], 10);

        if (Object.hasOwn(months, monthName)) {
          const date = new Date(year, months[monthName], day);
          if (!isNaN(date.getTime())) {
            return date.getTime();
          }
        }
      }

      // Try to match abbreviated format "Mon D-D" or "Mon D - D" (e.g., "Dec 1-12" or "Dec 1 - 12")
      const abbrevMatch = dateStr.match(/(\w{3})\s+(\d{1,2})\s*-\s*(\d{1,2})/);
      if (abbrevMatch) {
        const monthAbbrev = abbrevMatch[1];
        const startDay = parseInt(abbrevMatch[2], 10);
        // Use current year since it's not provided
        const currentYear = new Date().getFullYear();

        if (Object.hasOwn(months, monthAbbrev)) {
          const date = new Date(currentYear, months[monthAbbrev], startDay);
          if (!isNaN(date.getTime())) {
            return date.getTime();
          }
        }
      }

      // Try to match "Month YYYY" format (e.g., "May 2016", "November 2025")
      // IMPORTANT: Must come BEFORE "Mon D" to prevent "2016" being parsed as day 20
      const monthYearMatch = dateStr.match(/^(\w+)\s+(\d{4})$/);
      if (monthYearMatch) {
        const monthName = monthYearMatch[1];
        const year = parseInt(monthYearMatch[2], 10);

        if (Object.hasOwn(months, monthName)) {
          // Use the 1st day of the month for sorting (earliest possible date in that month)
          const date = new Date(year, months[monthName], 1);
          if (!isNaN(date.getTime())) {
            return date.getTime();
          }
        }
      }

      // Try to match "Mon D" format (e.g., "Dec 1")
      const singleDayMatch = dateStr.match(/(\w{3})\s+(\d{1,2})(?!\s*-)/);
      if (singleDayMatch) {
        const monthAbbrev = singleDayMatch[1];
        const day = parseInt(singleDayMatch[2], 10);
        const currentYear = new Date().getFullYear();

        if (Object.hasOwn(months, monthAbbrev)) {
          const date = new Date(currentYear, months[monthAbbrev], day);
          if (!isNaN(date.getTime())) {
            return date.getTime();
          }
        }
      }

      // Fallback: try parsing the date string directly
      const parsed = new Date(dateStr);
      if (!isNaN(parsed.getTime())) {
        return parsed.getTime();
      }
      return 0;
    } catch {
      return 0;
    }
  };

  // Derived from the window rather than read off a stored field, so a badge
  // whose earn period opens while the gallery is open reclassifies itself.
  const getBadgeStatus = (badge: BadgeWithMetadata) =>
    deriveBadgeStatus(
      badge.badgebase_info?.more_info,
      badge.badgebase_info?.enrichment as Record<string, unknown> | undefined
    );

  const isBadgeAvailable = (badge: BadgeWithMetadata): boolean => {
    return getBadgeStatus(badge) === 'available';
  };

  const isBadgeComingSoon = (badge: BadgeWithMetadata): boolean => {
    return getBadgeStatus(badge) === 'coming-soon';
  };

  // Sort badges based on selected option - use useMemo to prevent re-sorting on every render
  const sortedBadges = useMemo(() => {
    Logger.debug(`[BadgesOverlay] Sorting ${badgesWithMetadata.length} badges by ${sortBy}`);

    // Check if we can use pre-computed positions for date-newest sort
    // Only use positions if at least 90% of badges have them (to handle edge cases)
    const badgesWithPositions = badgesWithMetadata.filter(b =>
      b.badgebase_info && typeof (b.badgebase_info as any).position === 'number'
    ).length;

    const canUsePositions = sortBy === 'date-newest' &&
      badgesWithMetadata.length > 0 &&
      badgesWithPositions >= badgesWithMetadata.length * 0.9;

    if (canUsePositions) {
      Logger.debug(`[BadgesOverlay] Using pre-computed positions for sorting (${badgesWithPositions}/${badgesWithMetadata.length} badges have positions)`);
      return [...badgesWithMetadata].sort((a, b) => {
        const aPos = (a.badgebase_info as any)?.position;
        const bPos = (b.badgebase_info as any)?.position;

        // If both have positions, use them
        if (typeof aPos === 'number' && typeof bPos === 'number') {
          return aPos - bPos;
        }

        // If only one has a position, sort by date for fair comparison
        const dateCompare = parseDate(b.badgebase_info?.date_added) - parseDate(a.badgebase_info?.date_added);
        if (dateCompare !== 0) return dateCompare;

        // Fallback to stable sort
        return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
      });
    }

    // Log sample badge data for debugging
    if (badgesWithMetadata.length > 0) {
      const sample = badgesWithMetadata[0];
      Logger.debug('[BadgesOverlay] Sample badge:', {
        set_id: sample.set_id,
        id: sample.id,
        title: sample.title,
        date_added: sample.badgebase_info?.date_added,
        usage_stats: sample.badgebase_info?.usage_stats,
        more_info: sample.badgebase_info?.more_info
      });
    }

    return [...badgesWithMetadata].sort((a, b) => {
      switch (sortBy) {
        case 'available': {
          // Available badges first, then by newest
          const aAvailable = isBadgeAvailable(a) ? 1 : 0;
          const bAvailable = isBadgeAvailable(b) ? 1 : 0;
          if (aAvailable !== bAvailable) {
            return bAvailable - aAvailable;
          }
          // Secondary sort by date
          const dateCompare = parseDate(b.badgebase_info?.date_added) - parseDate(a.badgebase_info?.date_added);
          if (dateCompare !== 0) return dateCompare;
          // Tertiary sort by set_id and id for stability
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
        }
        case 'coming-soon': {
          // Coming soon badges first, then by newest
          const aComingSoon = isBadgeComingSoon(a) ? 1 : 0;
          const bComingSoon = isBadgeComingSoon(b) ? 1 : 0;
          if (aComingSoon !== bComingSoon) {
            return bComingSoon - aComingSoon;
          }
          // Secondary sort by date
          const dateCompare = parseDate(b.badgebase_info?.date_added) - parseDate(a.badgebase_info?.date_added);
          if (dateCompare !== 0) return dateCompare;
          // Tertiary sort by set_id and id for stability
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
        }
        case 'date-newest': {
          const dateCompare = parseDate(b.badgebase_info?.date_added) - parseDate(a.badgebase_info?.date_added);
          if (dateCompare !== 0) return dateCompare;
          // Fallback to stable sort
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
        }
        case 'date-oldest': {
          const dateCompare = parseDate(a.badgebase_info?.date_added) - parseDate(b.badgebase_info?.date_added);
          if (dateCompare !== 0) return dateCompare;
          // Fallback to stable sort
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
        }
        case 'usage-high': {
          const usageCompare = parseUsageStats(b.badgebase_info?.usage_stats) - parseUsageStats(a.badgebase_info?.usage_stats);
          if (usageCompare !== 0) return usageCompare;
          // Fallback to stable sort
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
        }
        case 'usage-low': {
          const usageCompare = parseUsageStats(a.badgebase_info?.usage_stats) - parseUsageStats(b.badgebase_info?.usage_stats);
          if (usageCompare !== 0) return usageCompare;
          // Fallback to stable sort
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
        }
        default:
          return `${a.set_id}-${a.id}`.localeCompare(`${b.set_id}-${b.id}`);
      }
    });
  }, [badgesWithMetadata, sortBy]);

  // Search filter for the Twitch tab (the search box previously did nothing
  // here). Also lets a profile badge click deep-link to a specific Twitch badge
  // by setting the query to its title.
  const displayedTwitchBadges = useMemo(() => {
    let list = sortedBadges;
    // "Available Now" / "Coming Soon" are filters, not just orderings: narrow the
    // list to matching badges rather than floating them above everything else.
    if (sortBy === 'available') {
      list = list.filter(isBadgeAvailable);
    } else if (sortBy === 'coming-soon') {
      list = list.filter(isBadgeComingSoon);
    }
    const q = searchQuery.trim().toLowerCase();
    if (q) {
      list = list.filter(b => (b.title || '').toLowerCase().includes(q));
    }
    return list;
  }, [sortedBadges, sortBy, searchQuery]);

  // Message for the Twitch tab when the active view has no badges. The
  // status filters can legitimately match nothing (e.g. no badge is currently
  // mid-event), so explain that rather than implying the catalog is empty.
  const twitchEmptyMessage =
    sortBy === 'available' ? 'No badges are available to collect right now.' :
    sortBy === 'coming-soon' ? 'No upcoming badges have been announced yet.' :
    'No badges found';

  return (
    <motion.div 
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.15 }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-2xl"
    >
      {/* Hover-sensitive background overlay */}
      <div
        className="absolute inset-0 group-hover:pointer-events-none"
        onClick={onClose}
      />

      <motion.div 
        initial={{ opacity: 0, scale: 0.95, y: 15 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.95, y: 15 }}
        transition={{ type: "spring", stiffness: 350, damping: 25 }}
        style={{ willChange: "transform, opacity" }}
        // sn-light-off: a cosmetics stage; a theme's light treatment stays off
        // it so a badge or atmosphere is seen on neutral glass.
        className="sn-light-off liquid-glass-panel w-[90vw] h-[85vh] max-w-7xl flex flex-col relative z-10 overflow-hidden"
      >
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-borderSubtle">
          <div className="flex items-center gap-4">
            {/* Tab Navigation */}
            <div className="flex items-center gap-2">
              <button
                onClick={() => setActiveTab('twitch-badges')}
                className={`px-4 py-2 rounded-lg transition-all flex items-center gap-2 ${
                  activeTab === 'twitch-badges'
                    ? 'glass-button text-accent shadow-[0_0_15px_rgba(var(--color-accent-rgb),0.25)]'
                    : 'text-textSecondary hover:text-textPrimary'
                }`}
              >
                <svg className="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z"/>
                </svg>
                Badges
                <span className="text-xs opacity-70">({badgesWithMetadata.length})</span>
              </button>
              
              <button
                onClick={() => setActiveTab('7tv-badges')}
                className={`px-4 py-2 rounded-lg transition-all flex items-center gap-2 ${
                  activeTab === '7tv-badges'
                    ? 'glass-button text-[#29b6f6] shadow-[0_0_15px_rgba(41,182,246,0.3)]'
                    : 'text-textSecondary hover:text-textPrimary'
                }`}
              >
                <svg className="w-4 h-4" viewBox="0 0 28 21" fill="currentColor">
                  <path d="M20.7465 5.48825L21.9799 3.33745L22.646 2.20024L21.4125 0.0494437V0H14.8259L17.2928 4.3016L17.9836 5.48825H20.7465Z" />
                  <path d="M7.15395 19.9258L14.5546 7.02104L15.4673 5.43884L13.0004 1.13724L12.3097 0.0247596H1.8995L0.666057 2.17556L0 3.31276L1.23344 5.46356V5.51301H9.12745L2.96025 16.267L2.09685 17.7998L3.33029 19.9506V20H7.15395" />
                  <path d="M17.4655 19.9257H21.2398L26.1736 11.3225L27.037 9.83924L25.8036 7.68844V7.63899H22.0046L19.5377 11.9406L19.365 12.262L16.8981 7.96038L16.7255 7.63899L14.2586 11.9406L13.5679 13.1272L17.2682 19.5796L17.4655 19.9257Z" />
                </svg>
                Badges
                <span className="text-xs opacity-70">({loadingSeventvBadges ? '...' : seventvBadges.length})</span>
              </button>
              
              <button
                onClick={() => setActiveTab('7tv-paints')}
                className={`px-4 py-2 rounded-lg transition-all flex items-center gap-2 ${
                  activeTab === '7tv-paints'
                    ? 'glass-button text-[#29b6f6] shadow-[0_0_15px_rgba(41,182,246,0.3)]'
                    : 'text-textSecondary hover:text-textPrimary'
                }`}
              >
                <svg className="w-4 h-4" viewBox="0 0 28 21" fill="currentColor">
                  <path d="M20.7465 5.48825L21.9799 3.33745L22.646 2.20024L21.4125 0.0494437V0H14.8259L17.2928 4.3016L17.9836 5.48825H20.7465Z" />
                  <path d="M7.15395 19.9258L14.5546 7.02104L15.4673 5.43884L13.0004 1.13724L12.3097 0.0247596H1.8995L0.666057 2.17556L0 3.31276L1.23344 5.46356V5.51301H9.12745L2.96025 16.267L2.09685 17.7998L3.33029 19.9506V20H7.15395" />
                  <path d="M17.4655 19.9257H21.2398L26.1736 11.3225L27.037 9.83924L25.8036 7.68844V7.63899H22.0046L19.5377 11.9406L19.365 12.262L16.8981 7.96038L16.7255 7.63899L14.2586 11.9406L13.5679 13.1272L17.2682 19.5796L17.4655 19.9257Z" />
                </svg>
                Paints
                <span className="text-xs opacity-70">({loadingSeventvPaints ? '...' : seventvPaints.length})</span>
              </button>

              <button
                onClick={() => setActiveTab('streamnook')}
                className={`px-4 py-2 rounded-lg transition-all flex items-center gap-2 ${
                  activeTab === 'streamnook'
                    ? 'glass-button text-accent shadow-[0_0_15px_rgba(var(--color-accent-rgb),0.25)]'
                    : 'text-textSecondary hover:text-textPrimary'
                }`}
              >
                <img src={streamNookLogo} alt="" className="w-4 h-4 object-contain" draggable={false} />
                StreamNook
              </button>

              <button
                onClick={() => setActiveTab('bttv')}
                className={`px-4 py-2 rounded-lg transition-all flex items-center gap-2 ${
                  activeTab === 'bttv'
                    ? 'glass-button text-[#e84b4b] shadow-[0_0_15px_rgba(232,75,75,0.3)]'
                    : 'text-textSecondary hover:text-textPrimary'
                }`}
              >
                <img src={betterttvLogo} alt="" className="w-4 h-4 object-contain" draggable={false} />
                BetterTTV
                <span className="text-xs opacity-70">({loadingChatClientBadges ? '...' : chatClientBadges.filter(b => b.provider === 'bttv').length + (myBttvProBadge ? 1 : 0)})</span>
              </button>

              <button
                onClick={() => setActiveTab('chat-clients')}
                className={`px-4 py-2 rounded-lg transition-all flex items-center gap-2 ${
                  activeTab === 'chat-clients'
                    ? 'glass-button text-accent shadow-[0_0_15px_rgba(var(--color-accent-rgb),0.25)]'
                    : 'text-textSecondary hover:text-textPrimary'
                }`}
              >
                <img src={chatterinoLogo} alt="" className="w-4 h-4 object-contain" draggable={false} />
                Chat Clients
                <span className="text-xs opacity-70">({loadingChatClientBadges ? '...' : chatClientBadges.filter(b => b.provider !== 'bttv').length})</span>
              </button>

              {/* Search Input. Hidden on StreamNook tab (static content, nothing to search) */}
              {activeTab !== 'streamnook' && (
                <div className="relative ml-auto">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-textSecondary" />
                  <input
                    type="text"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder={`Search ${activeTab === 'twitch-badges' ? 'badges' : activeTab === '7tv-badges' ? '7TV badges' : activeTab === 'bttv' ? 'BetterTTV badges' : activeTab === 'chat-clients' ? 'chat-client badges' : '7TV paints'}...`}
                    className="w-48 pl-9 pr-3 py-2 glass-input text-sm text-textPrimary placeholder-textSecondary focus:outline-none transition-colors"
                  />
                </div>
              )}
            </div>
            
            {/* Collection Counter - shows collected/total for current tab */}
            {isAuthenticated && (
              <>
                {/* Twitch Badges Counter */}
                {activeTab === 'twitch-badges' && totalGlobalBadges > 0 && (
                  <div className="flex items-center gap-2 px-3 py-1.5 glass-badge">
                    <Check size={14} className="text-green-400" />
                    {loadingUserBadges ? (
                      <div className="w-4 h-4 border-2 border-t-transparent border-accent rounded-full animate-spin" />
                    ) : (
                      <span className="text-sm text-textPrimary">
                        <span className="font-semibold text-accent">{collectedCount}</span>
                        <span className="text-textSecondary"> / {totalGlobalBadges} collected</span>
                      </span>
                    )}
                  </div>
                )}
                
                {/* 7TV Badges Counter */}
                {activeTab === '7tv-badges' && seventvBadges.length > 0 && (
                  <div className="flex items-center gap-2 px-3 py-1.5 glass-badge">
                    <Check size={14} className="text-[#29b6f6]" />
                    {loadingUser7TVCosmetics ? (
                      <div className="w-4 h-4 border-2 border-t-transparent border-[#29b6f6] rounded-full animate-spin" />
                    ) : (
                      <span className="text-sm text-textPrimary">
                        <span className="font-semibold text-[#29b6f6]">{userOwned7TVBadgeIds.size}</span>
                        <span className="text-textSecondary"> / {seventvBadges.length} owned</span>
                      </span>
                    )}
                  </div>
                )}
                
                {/* 7TV Paints Counter */}
                {activeTab === '7tv-paints' && seventvPaints.length > 0 && (
                  <div className="flex items-center gap-2 px-3 py-1.5 glass-badge">
                    <Check size={14} className="text-[#29b6f6]" />
                    {loadingUser7TVCosmetics ? (
                      <div className="w-4 h-4 border-2 border-t-transparent border-[#29b6f6] rounded-full animate-spin" />
                    ) : (
                      <span className="text-sm text-textPrimary">
                        <span className="font-semibold text-[#29b6f6]">{userOwned7TVPaintIds.size}</span>
                        <span className="text-textSecondary"> / {seventvPaints.length} owned</span>
                      </span>
                    )}
                  </div>
                )}

                {/* Chat Clients Counter */}
                {activeTab === 'chat-clients' && chatClientBadges.length > 0 && (
                  <div className="flex items-center gap-2 px-3 py-1.5 glass-badge">
                    <Check size={14} className="text-accent" />
                    <span className="text-sm text-textPrimary">
                      <span className="font-semibold text-accent">{chatClientBadges.filter(b => b.owned).length}</span>
                      <span className="text-textSecondary"> / {chatClientBadges.length} owned</span>
                    </span>
                  </div>
                )}
              </>
            )}
          </div>
          <Tooltip content="Close" side="bottom">
            <button
              onClick={onClose}
              className="p-2 hover:bg-white/5 rounded-lg transition-colors"
            >
              <X size={20} className="text-textSecondary" />
            </button>
          </Tooltip>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 custom-scrollbar">
          {/* ========== TWITCH BADGES TAB ========== */}
          {activeTab === 'twitch-badges' && (
            <>
              {loading && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-accent mx-auto mb-4"></div>
                    <p className="text-textSecondary">Loading badges...</p>
                  </div>
                </div>
              )}

              {error && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <p className="text-red-400 mb-4">{error}</p>
                    <button
                      onClick={loadBadges}
                      className="px-4 py-2 glass-button text-white"
                    >
                      Retry
                    </button>
                  </div>
                </div>
              )}

          {!loading && !error && (
            <>
              {/* Sort Controls */}
              <div className="flex items-center gap-3 mb-6">
                <div className="flex items-center gap-2 text-textSecondary">
                  <ArrowUpDown size={16} />
                  <span className="text-sm font-medium">Sort by:</span>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => setSortBy('date-newest')}
                    className={`px-3 py-1.5 rounded-lg text-sm transition-colors ${sortBy === 'date-newest'
                      ? 'glass-button text-accent shadow-[0_0_10px_rgba(var(--color-accent-rgb),0.3)]'
                      : 'hover:bg-white/5 text-textSecondary hover:text-white'
                      }`}
                  >
                    Newest First
                  </button>
                  <button
                    onClick={() => setSortBy('date-oldest')}
                    className={`px-3 py-1.5 rounded-lg text-sm transition-colors ${sortBy === 'date-oldest'
                      ? 'glass-button text-accent shadow-[0_0_10px_rgba(var(--color-accent-rgb),0.3)]'
                      : 'hover:bg-white/5 text-textSecondary hover:text-white'
                      }`}
                  >
                    Oldest First
                  </button>
                  <button
                    onClick={() => setSortBy('usage-high')}
                    className={`px-3 py-1.5 rounded-lg text-sm transition-colors ${sortBy === 'usage-high'
                      ? 'glass-button text-accent shadow-[0_0_10px_rgba(var(--color-accent-rgb),0.3)]'
                      : 'hover:bg-white/5 text-textSecondary hover:text-white'
                      }`}
                  >
                    Most Used
                  </button>
                  <button
                    onClick={() => setSortBy('usage-low')}
                    className={`px-3 py-1.5 rounded-lg text-sm transition-colors ${sortBy === 'usage-low'
                      ? 'glass-button text-accent shadow-[0_0_10px_rgba(var(--color-accent-rgb),0.3)]'
                      : 'hover:bg-white/5 text-textSecondary hover:text-white'
                      }`}
                  >
                    Least Used
                  </button>
                  <button
                    onClick={() => setSortBy('available')}
                    className={`px-3 py-1.5 rounded-lg text-sm transition-colors flex items-center gap-2 ${sortBy === 'available'
                      ? 'glass-button text-green-400 border-green-500/30 shadow-[0_0_10px_color-mix(in_srgb,var(--color-success)_30%,transparent)]'
                      : 'hover:bg-white/5 text-textSecondary hover:text-white'
                      }`}
                  >
                    <span className="w-2 h-2 bg-green-500 rounded-full animate-pulse"></span>
                    Available Now
                  </button>
                  <button
                    onClick={() => setSortBy('coming-soon')}
                    className={`px-3 py-1.5 rounded-lg text-sm transition-colors flex items-center gap-2 ${sortBy === 'coming-soon'
                      ? 'glass-button text-blue-400 border-blue-500/30 shadow-[0_0_10px_color-mix(in_srgb,var(--color-info)_30%,transparent)]'
                      : 'hover:bg-white/5 text-textSecondary hover:text-white'
                      }`}
                  >
                    <span className="w-2 h-2 bg-blue-500 rounded-full animate-pulse"></span>
                    Coming Soon
                  </button>
                </div>
                {loadingMetadata && (
                  <div className="ml-auto flex items-center gap-2 text-textSecondary text-sm">
                    <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-accent"></div>
                    <span>Loading badge data...</span>
                  </div>
                )}
                {newBadgesCount > 0 && !loadingMetadata && (
                  <div className="ml-auto flex items-center gap-2 text-yellow-500 text-sm">
                    <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-yellow-500"></div>
                    <span>Fetching {newBadgesCount} new badge{newBadgesCount !== 1 ? 's' : ''}...</span>
                  </div>
                )}
                {!loadingMetadata && newBadgesCount === 0 && (
                  <div className="ml-auto flex items-center gap-2">
                    {cacheAge !== null && cacheAge > 0 && (
                      <span className="text-textSecondary text-xs">
                        Cache age: {cacheAge} day{cacheAge !== 1 ? 's' : ''}
                      </span>
                    )}
                    <Tooltip content="Force refresh badges from Twitch API" side="top">
                      <button
                        onClick={forceRefreshBadges}
                        disabled={refreshing}
                        className="flex items-center gap-1 px-2 py-1 glass-button-static rounded text-xs text-textSecondary hover:text-textPrimary transition-colors disabled:opacity-50"
                      >
                        <RefreshCw size={12} className={refreshing ? 'animate-spin' : ''} />
                        {refreshing ? 'Refreshing...' : 'Refresh'}
                      </button>
                    </Tooltip>
                  </div>
                )}
              </div>

              {displayedTwitchBadges.length === 0 && (
                <div className="flex items-center justify-center py-20">
                  <p className="text-textSecondary">{twitchEmptyMessage}</p>
                </div>
              )}

              {/* Badge Grid */}
              <div className="grid grid-cols-8 gap-6">
                {displayedTwitchBadges.map((badge, index) => {
                  const isAvailable = isBadgeAvailable(badge);
                  const isComingSoon = isBadgeComingSoon(badge);
                  const hasCollected = isAuthenticated && isCollected(badge);
                  return (
                    <Tooltip key={`${badge.set_id}-${badge.id}-${index}`} content={hasCollected ? `${badge.title} (Collected!)` : badge.title} side="bottom">
                      <button
                        onClick={() => onBadgeClick(badge, badge.set_id)}
                        className={`flex flex-col items-center gap-2 p-3 transition-all duration-300 group relative ${
                          hasCollected ? 'rounded-xl bg-gradient-to-br from-[#d4a84b]/15 to-[#b8860b]/5 border border-[#d4a84b]/30 shadow-[inset_0_1px_1px_rgba(255,255,255,0.1),0_4px_20px_rgba(212,168,75,0.15)] backdrop-blur-md hover:bg-[#d4a84b]/20 hover:border-[#d4a84b]/50' :
                          isAvailable ? 'rounded-lg hover:bg-white/5 ring-2 ring-green-500/50' : 
                          isComingSoon ? 'rounded-lg hover:bg-white/5 ring-2 ring-blue-500/50' : 'rounded-lg hover:bg-white/5'
                        }`}
                      >
                      <div className={`w-18 h-18 flex items-center justify-center bg-transparent group-hover:scale-110 transition-transform duration-300 relative ${
                        hasCollected ? 'drop-shadow-[0_0_15px_rgba(212,168,75,0.4)]' :
                        isAvailable ? 'drop-shadow-[0_0_15px_rgba(34,197,94,0.4)]' :
                        isComingSoon ? 'drop-shadow-[0_0_15px_rgba(59,130,246,0.4)]' : ''
                      }`}>
                        <img
                          src={badge.image_url_4x}
                          alt={badge.title}
                          className="w-16 h-16 object-contain"
                          loading="lazy"
                        />
                      </div>
                      {/* Collected indicator - takes priority over other indicators */}
                      {hasCollected && (
                        <div className="absolute -top-1 -right-1 w-6 h-6 rounded-full flex items-center justify-center bg-black/40 backdrop-blur-md border border-[#d4a84b]/60 shadow-[0_0_15px_rgba(212,168,75,0.4)] overflow-hidden">
                          <div className="absolute inset-0 bg-gradient-to-br from-[#d4a84b]/30 to-transparent pointer-events-none" />
                          <Check size={13} className="text-[#f0d78c] drop-shadow-[0_0_3px_rgba(212,168,75,0.8)] z-10" strokeWidth={2.5} />
                        </div>
                      )}
                      {/* Status indicators - only show if not collected */}
                      {!hasCollected && isAvailable && (
                        <span className="absolute top-1 right-1 w-2 h-2 bg-green-500 rounded-full animate-pulse"></span>
                      )}
                      {!hasCollected && isComingSoon && (
                        <span className="absolute top-1 right-1 w-2 h-2 bg-blue-500 rounded-full animate-pulse"></span>
                      )}
                      <span className={`text-xs text-center line-clamp-2 transition-all duration-300 font-medium ${
                        hasCollected ? 'text-[#f0d78c] drop-shadow-[0_0_8px_rgba(212,168,75,0.3)]' : 'text-textSecondary group-hover:text-textPrimary'
                      }`}>
                        {badge.title}
                      </span>
                    </button>
                    </Tooltip>
                  );
                })}
              </div>
            </>
          )}
            </>
          )}

          {/* ========== 7TV BADGES TAB ========== */}
          {activeTab === '7tv-badges' && (
            <>
              {loadingSeventvBadges && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-[#29b6f6] mx-auto mb-4"></div>
                    <p className="text-textSecondary">Loading 7TV badges...</p>
                  </div>
                </div>
              )}

              {seventvBadgesError && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <p className="text-red-400 mb-4">{seventvBadgesError}</p>
                    <button
                      onClick={() => { setSeventvBadges([]); loadSeventvBadges(); }}
                      className="px-4 py-2 glass-button text-[#29b6f6]"
                    >
                      Retry
                    </button>
                  </div>
                </div>
              )}

              {!loadingSeventvBadges && !seventvBadgesError && seventvBadges.length === 0 && (
                <div className="flex items-center justify-center h-full">
                  <p className="text-textSecondary">No 7TV badges found</p>
                </div>
              )}

              {!loadingSeventvBadges && !seventvBadgesError && seventvBadges.length > 0 && (
                <>
                  {/* Sort Controls */}
                  <div className="flex items-center justify-between mb-4">
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-textSecondary">Sort:</span>
                      <button
                        onClick={() => setSeventvBadgeSortBy('newest')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvBadgeSortBy === 'newest' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Newest First
                      </button>
                      <button
                        onClick={() => setSeventvBadgeSortBy('oldest')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvBadgeSortBy === 'oldest' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Oldest First
                      </button>
                      <button
                        onClick={() => setSeventvBadgeSortBy('name')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvBadgeSortBy === 'name' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        A-Z
                      </button>
                    </div>
                    <span className="text-xs text-textSecondary">
                      {sortedSeventvBadges.length} badge{sortedSeventvBadges.length !== 1 ? 's' : ''}
                    </span>
                  </div>

                  {/* Badge Grid */}
                  <div className="grid grid-cols-8 gap-2">
                    {sortedSeventvBadges.map((badge) => {
                    const animated = isAnimatedBadge(badge);
                    const isOwned = userOwned7TVBadgeIds.has(badge.id);
                    return (
                      <Tooltip key={badge.id} content={badge.description || badge.name}>
                      <button
                        onClick={() => setSelectedSeventvBadge(badge)}
                        className={`flex flex-col items-center gap-2 p-3 rounded-lg hover:bg-white/5 transition-all duration-200 group cursor-pointer relative ${
                          isOwned ? 'ring-2 ring-[#29b6f6]/50 bg-[#29b6f6]/10' : ''
                        }`}
                      >
                        {/* Owned indicator */}
                        {isOwned && (
                          <div className="absolute top-1 right-1 w-5 h-5 bg-[#29b6f6] rounded-full flex items-center justify-center shadow-lg z-10">
                            <Check size={12} className="text-white" />
                          </div>
                        )}
                        <div className="w-18 h-18 flex items-center justify-center bg-transparent group-hover:scale-110 transition-transform duration-200">
                          <img
                            src={getSeventvBadgeImageUrl(badge)}
                            alt={badge.name}
                            className="w-16 h-16 object-contain"
                            loading="lazy"
                          />
                        </div>
                        <span className={`text-xs text-center line-clamp-2 transition-colors font-medium ${
                          isOwned ? 'text-[#29b6f6]' : 'text-textSecondary group-hover:text-textPrimary'
                        }`}>
                          {badge.name}
                        </span>

                      </button>
                      </Tooltip>
                    );
                  })}
                  </div>
                </>
              )}
            </>
          )}

          {/* ========== 7TV PAINTS TAB ========== */}
          {activeTab === '7tv-paints' && (
            <>
              {loadingSeventvPaints && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-[#29b6f6] mx-auto mb-4"></div>
                    <p className="text-textSecondary">Loading 7TV paints...</p>
                  </div>
                </div>
              )}

              {seventvPaintsError && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <p className="text-red-400 mb-4">{seventvPaintsError}</p>
                    <button
                      onClick={() => { setSeventvPaints([]); loadSeventvPaints(); }}
                      className="px-4 py-2 glass-button text-[#29b6f6]"
                    >
                      Retry
                    </button>
                  </div>
                </div>
              )}

              {!loadingSeventvPaints && !seventvPaintsError && seventvPaints.length === 0 && (
                <div className="flex items-center justify-center h-full">
                  <p className="text-textSecondary">No 7TV paints found</p>
                </div>
              )}

              {!loadingSeventvPaints && !seventvPaintsError && seventvPaints.length > 0 && (
                <>
                  {/* Sort Controls */}
                  <div className="flex items-center justify-between mb-4">
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-textSecondary">Sort:</span>
                      <button
                        onClick={() => setSeventvPaintSortBy('newest')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintSortBy === 'newest' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Newest First
                      </button>
                      <button
                        onClick={() => setSeventvPaintSortBy('oldest')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintSortBy === 'oldest' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Oldest First
                      </button>
                      <button
                        onClick={() => setSeventvPaintSortBy('name')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintSortBy === 'name'
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]'
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        A-Z
                      </button>
                      <button
                        onClick={() => setSeventvPaintSortBy('most-used')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintSortBy === 'most-used'
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]'
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Most Used
                      </button>
                      <button
                        onClick={() => setSeventvPaintSortBy('least-used')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintSortBy === 'least-used'
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]'
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Least Used
                      </button>

                      {/* Separator */}
                      <div className="w-px h-4 bg-borderSubtle mx-1" />
                      
                      {/* Filter buttons */}
                      <span className="text-xs text-textSecondary">Filter:</span>
                      <button
                        onClick={() => setSeventvPaintFilter('all')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintFilter === 'all' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        All
                      </button>
                      <button
                        onClick={() => setSeventvPaintFilter('animated')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintFilter === 'animated' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Animated
                      </button>
                      <button
                        onClick={() => setSeventvPaintFilter('static')}
                        className={`px-3 py-1.5 text-xs rounded-lg transition-colors ${
                          seventvPaintFilter === 'static' 
                            ? 'glass-button text-[#29b6f6] shadow-[0_0_10px_rgba(41,182,246,0.3)]' 
                            : 'hover:bg-white/5 text-textSecondary hover:text-white'
                        }`}
                      >
                        Static
                      </button>
                    </div>
                    <span className="text-xs text-textSecondary">
                      {sortedSeventvPaints.length} paint{sortedSeventvPaints.length !== 1 ? 's' : ''}
                    </span>
                  </div>

                  {/* Paint Grid */}
                  <div className="grid grid-cols-6 gap-4">
                    {sortedSeventvPaints.map((paint) => {
                      const isOwned = userOwned7TVPaintIds.has(paint.id);
                    
                      return (
                        <Tooltip key={paint.id} content={paint.description || paint.name}>
                        <button
                          onClick={() => setSelectedSeventvPaint(paint)}
                          className={`flex flex-col items-center gap-2 p-3 rounded-lg hover:bg-white/5 transition-all duration-200 group cursor-pointer relative ${
                            isOwned ? 'ring-2 ring-[#29b6f6]/50 bg-[#29b6f6]/10' : ''
                          }`}
                          style={{ contentVisibility: 'auto', containIntrinsicSize: 'auto 100px' }}
                        >
                          {/* Owned indicator */}
                          {isOwned && (
                            <div className="absolute top-1 right-1 w-5 h-5 bg-[#29b6f6] rounded-full flex items-center justify-center shadow-lg z-10">
                              <Check size={12} className="text-white" />
                            </div>
                          )}
                          <div
                            className="w-full h-14 flex items-center justify-center rounded-lg group-hover:scale-110 transition-transform duration-200 overflow-hidden bg-transparent relative"
                          >
                            {/* Use CSS background-image approach for ALL paint types per 7TV docs */}
                            <span 
                              className="text-base font-bold px-2 truncate relative"
                              style={{ 
                                background: generatePaintGradient(paint, 1),
                                backgroundSize: '100% 100%',
                                backgroundClip: 'text',
                                WebkitBackgroundClip: 'text',
                                WebkitTextFillColor: 'transparent',
                                filter: generatePaintShadow(paint)
                              }}
                            >
                              {paint.name}
                            </span>
                          </div>
                          <span className={`text-xs text-center line-clamp-2 transition-colors font-medium ${
                            isOwned ? 'text-[#29b6f6]' : 'text-textSecondary group-hover:text-textPrimary'
                          }`}>
                            {paint.name}
                          </span>
                        </button>
                        </Tooltip>
                      );
                    })}
                  </div>
                </>
              )}
            </>
          )}

          {/* StreamNook Tab. The app's own identity badge with static content
              (description + tier breakdown + viewer's own card if signed in).
              Not data-driven from any API; tiers are durable per the Brain. */}
          {activeTab === 'streamnook' && (
            <div className="max-w-3xl mx-auto py-6 space-y-8">
              {/* Identity hero: wordmark + viewer's tier card + subtle blurb.
                  Identity-related content lives at the top so the page reads
                  as "who you are in StreamNook" before "what badges exist". */}
              <div className="flex flex-col items-center text-center">
                <img
                  src={streamNookLogo}
                  alt="StreamNook"
                  className="w-16 h-16 object-contain mb-3"
                  draggable={false}
                />
                <h2 className="text-3xl font-bold text-textPrimary mb-2">StreamNook</h2>
                <p className="text-sm text-textSecondary uppercase tracking-[0.28em] font-medium mb-5">
                  Community Identity Badge
                </p>
                {currentUserStreamNookNumber !== null && (
                  <StreamNookTierCard userNumber={currentUserStreamNookNumber} skipCypher />
                )}
                <p className="mt-5 max-w-md text-[12px] leading-relaxed text-textSecondary/70">
                  A permanent rank by join order, with a tier label tied to it. Hover any badge in chat
                  to reveal the rank. Visible only to other members.
                </p>
              </div>

              {/* Sub-tabs: StreamNook's own cosmetics, split into Badges and
                  Atmospheres, so the top bar keeps one "StreamNook" entry. */}
              <div className="flex justify-center gap-2">
                <button
                  onClick={() => setSnTab('badges')}
                  className={`px-4 py-1.5 rounded-lg text-sm transition-all ${snTab === 'badges' ? 'glass-button text-violet-200' : 'text-textSecondary hover:text-textPrimary'}`}
                >
                  Badges
                </button>
                <button
                  onClick={() => setSnTab('atmospheres')}
                  className={`px-4 py-1.5 rounded-lg text-sm transition-all ${snTab === 'atmospheres' ? 'glass-button text-cyan-200' : 'text-textSecondary hover:text-textPrimary'}`}
                >
                  Atmospheres
                </button>
              </div>

              {/* Cosmetics catalog. Tile style ported verbatim from the Twitch
                  badges grid above so the two tabs read as the same surface
                  (gold "collected" treatment + corner Check). The equipped
                  cosmetic gets an additional accent ring on top of the gold. */}
              {snTab === 'badges' && cosmeticsCatalog.length > 0 && (
                <div>
                  <p className="text-[11px] text-textSecondary uppercase tracking-[0.28em] font-semibold mb-3 text-center">
                    Badges
                  </p>
                  <div className="grid grid-cols-3 gap-6 max-w-md mx-auto">
                    {cosmeticsCatalog.map((cosmetic) => {
                      // Grid tiles render at 64px; the detail modal below keeps
                      // the full-size asset for its 112px render.
                      const asset = resolveCosmeticAsset(cosmetic, { chatSize: true });
                      if (!asset) return null;
                      const owned = ownedCosmeticSlugs.has(cosmetic.slug);
                      const isActive = activeCosmeticSlug === cosmetic.slug;
                      return (
                        <Tooltip
                          key={cosmetic.slug}
                          content={
                            <div className="text-center">
                              <div className="font-semibold">{cosmetic.name}{owned ? ' · Collected' : ''}</div>
                              {cosmetic.description && (
                                <div className="text-[11px] text-textSecondary mt-0.5">{cosmetic.description}</div>
                              )}
                            </div>
                          }
                          side="bottom"
                        >
                          <button
                            onClick={() => setSelectedCosmetic(cosmetic)}
                            className={`flex flex-col items-center gap-2 p-3 transition-all duration-300 group relative ${
                              isActive ? 'rounded-xl bg-gradient-to-br from-[#d4a84b]/20 to-[#b8860b]/10 border border-[#d4a84b]/50 shadow-[inset_0_1px_1px_rgba(255,255,255,0.1),0_4px_20px_rgba(212,168,75,0.2)] backdrop-blur-md ring-2 ring-accent/50 hover:bg-[#d4a84b]/25 hover:border-[#d4a84b]/60' :
                              owned ? 'rounded-xl bg-gradient-to-br from-[#d4a84b]/15 to-[#b8860b]/5 border border-[#d4a84b]/30 shadow-[inset_0_1px_1px_rgba(255,255,255,0.1),0_4px_20px_rgba(212,168,75,0.15)] backdrop-blur-md hover:bg-[#d4a84b]/20 hover:border-[#d4a84b]/50' :
                              'rounded-lg hover:bg-white/5'
                            }`}
                          >
                            <div className={`w-18 h-18 flex items-center justify-center bg-transparent group-hover:scale-110 transition-transform duration-300 relative ${
                              owned ? 'drop-shadow-[0_0_15px_rgba(212,168,75,0.4)]' : ''
                            }`}>
                              <img
                                src={asset}
                                alt={cosmetic.name}
                                className="w-16 h-16 object-contain"
                                loading="lazy"
                              />
                            </div>
                            {owned && (
                              <div className="absolute -top-1 -right-1 w-6 h-6 rounded-full flex items-center justify-center bg-black/40 backdrop-blur-md border border-[#d4a84b]/60 shadow-[0_0_15px_rgba(212,168,75,0.4)] overflow-hidden">
                                <div className="absolute inset-0 bg-gradient-to-br from-[#d4a84b]/30 to-transparent pointer-events-none" />
                                <Check size={13} className="text-[#f0d78c] drop-shadow-[0_0_3px_rgba(212,168,75,0.8)] z-10" strokeWidth={2.5} />
                              </div>
                            )}
                            <span className={`text-xs text-center line-clamp-2 transition-all duration-300 font-medium ${
                              owned ? 'text-[#f0d78c] drop-shadow-[0_0_8px_rgba(212,168,75,0.3)]' : 'text-textSecondary group-hover:text-textPrimary'
                            }`}>
                              {cosmetic.name}
                            </span>
                          </button>
                        </Tooltip>
                      );
                    })}
                  </div>
                </div>
              )}

              {snTab === 'atmospheres' && (
                atmosphereLibrary.length === 0 ? (
                  <p className="py-10 text-center text-sm text-textMuted">Loading atmospheres...</p>
                ) : (
                  <div className="grid grid-cols-2 gap-4">
                    {atmosphereLibrary.map((a) => {
                      const unlock = getAtmosphereUnlock(a);
                      return (
                        <div key={a.id} className="overflow-hidden rounded-xl border border-white/[0.06] bg-white/[0.02]">
                          <div className="relative h-24 overflow-hidden">
                            <AtmosphereBackground atm={a} variant="profile" blur={!!a.image} />
                          </div>
                          <div className="flex items-center justify-between gap-2 p-3">
                            <span className="truncate text-sm font-medium text-textPrimary">{a.name}</span>
                            {unlock.hidden ? (
                              <Tooltip content="A secret. Keep using StreamNook to discover how to unlock it." side="top">
                                <span className="flex flex-shrink-0 items-center gap-1 rounded-full bg-white/[0.05] px-2 py-0.5 text-[10px] font-medium text-textMuted ring-1 ring-inset ring-white/10">
                                  <Lock size={10} /> Hidden challenge
                                </span>
                              </Tooltip>
                            ) : unlock.kind === 'subscriber' ? (
                              <Tooltip content="Included with a StreamNook subscription." side="top">
                                <span className="flex-shrink-0 rounded-full bg-violet-400/10 px-2 py-0.5 text-[10px] font-medium text-violet-200 ring-1 ring-inset ring-violet-400/20">
                                  Subscriber
                                </span>
                              </Tooltip>
                            ) : (
                              <Tooltip content={unlock.label} side="top">
                                <span className="flex-shrink-0 rounded-full bg-cyan-400/10 px-2 py-0.5 text-[10px] font-medium text-cyan-200 ring-1 ring-inset ring-cyan-400/20">
                                  {unlock.accoladeName} accolade
                                </span>
                              </Tooltip>
                            )}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )
              )}

              {/* Canonical "do something next" CTA. streamnook.app handles
                  Twitch sign-in + Stripe checkout with pre-attached identity. */}
              <div className="flex justify-center pt-2">
                <button
                  onClick={async () => {
                    try {
                      const { open } = await import('@tauri-apps/plugin-shell');
                      const userLogin = currentUser?.login || currentUser?.username;
                      const handle = userLogin ? `&handle=${encodeURIComponent(userLogin)}` : '';
                      await open(`https://streamnook.app/support?tier=subscriber${handle}`);
                    } catch (err) {
                      Logger.error('Failed to open support page:', err);
                    }
                  }}
                  className="px-5 py-2.5 glass-button text-sm text-textPrimary flex items-center gap-2 hover:bg-white/5 transition-colors"
                >
                  Subscribe
                  <ExternalLink size={14} className="opacity-60" />
                </button>
              </div>
            </div>
          )}

          {/* ========== CHAT CLIENTS TAB ========== */}
          {/* Browsable gallery of third-party chat-client badges (FFZ, BetterTTV,
              Chatterino, Homies, Chatsen, Chatty, DankChat), grouped into collapsible sections.
              These are assigned by each client to its devs/donors, so "owned" means
              the signed-in viewer's Twitch ID is in that badge's user list. */}
          {activeTab === 'bttv' && (
            <>
              {loadingChatClientBadges && (
                <div className="flex items-center justify-center h-full">
                  <div className="text-center">
                    <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-[#e84b4b] mx-auto mb-4"></div>
                    <p className="text-textSecondary">Loading BetterTTV badges...</p>
                  </div>
                </div>
              )}

              {!loadingChatClientBadges && (() => {
                // BetterTTV badges: the contributor badges from the public feed
                // (Developer / Translator / Emote Approver / Support) plus every
                // distinct Pro loyalty badge we've discovered across all users
                // (persisted server-side), shown to everyone regardless of who
                // owns them. The signed-in user's own badge is flagged owned.
                const ownUrl = myBttvProBadge?.url ?? null;
                const proUrls = Array.from(new Set([
                  ...discoveredProBadges,
                  ...(ownUrl ? [ownUrl] : []),
                ]));
                const proTiles = proUrls.map((url) => ({
                  id: `bttv-pro:${url}`, provider: 'bttv' as const, title: 'BTTV Pro',
                  image_1x: url, image_2x: url, image_4x: url,
                  user_count: 0, owned: url === ownUrl, click_url: 'https://betterttv.com',
                }));
                const all = [...chatClientBadges.filter(b => b.provider === 'bttv'), ...proTiles];
                const q = searchQuery.trim().toLowerCase();
                const badges = q ? all.filter(b => b.title.toLowerCase().includes(q)) : all;
                if (badges.length === 0) {
                  return (
                    <div className="flex items-center justify-center h-full">
                      <p className="text-textSecondary">No BetterTTV badges found</p>
                    </div>
                  );
                }
                return (
                  <div>
                    <div className="grid grid-cols-8 gap-2">
                      {badges.map((badge) => (
                        <Tooltip key={badge.id} content={badge.user_count > 0 ? `${badge.title} · ${badge.user_count} user${badge.user_count !== 1 ? 's' : ''}` : badge.title}>
                        <button
                          onClick={async () => {
                            if (!badge.click_url) return;
                            try {
                              const { open } = await import('@tauri-apps/plugin-shell');
                              await open(badge.click_url);
                            } catch (err) {
                              Logger.error('Failed to open badge link:', err);
                            }
                          }}
                          className={`flex flex-col items-center gap-2 p-3 rounded-lg hover:bg-white/5 transition-all duration-200 group cursor-pointer relative ${
                            badge.owned ? 'ring-2 ring-[#e84b4b]/50 bg-[#e84b4b]/10' : ''
                          }`}
                        >
                          {badge.owned && (
                            <div className="absolute top-1 right-1 w-5 h-5 bg-[#e84b4b] rounded-full flex items-center justify-center shadow-lg z-10">
                              <Check size={12} className="text-white" />
                            </div>
                          )}
                          <div className="w-18 h-18 flex items-center justify-center bg-transparent group-hover:scale-110 transition-transform duration-200">
                            <img
                              src={badge.image_4x || badge.image_2x || badge.image_1x}
                              alt={badge.title}
                              className="w-16 h-16 object-contain"
                              loading="lazy"
                            />
                          </div>
                          <span className={`text-xs text-center line-clamp-2 transition-colors font-medium ${
                            badge.owned ? 'text-[#e84b4b]' : 'text-textSecondary group-hover:text-textPrimary'
                          }`}>
                            {badge.title}
                          </span>
                        </button>
                        </Tooltip>
                      ))}
                    </div>
                  </div>
                );
              })()}
            </>
          )}

          {activeTab === 'chat-clients' && (
            <ChatClientsGallery
              badges={chatClientBadges}
              query={searchQuery}
              loading={loadingChatClientBadges}
              error={chatClientBadgesError}
              onRetry={() => { setChatClientBadges([]); loadChatClientBadges(); }}
            />
          )}
        </div>
      </motion.div>

      {/* 7TV Badge Detail Modal */}
      {selectedSeventvBadge && createPortal(
        <div 
          className="fixed inset-0 flex items-center justify-center bg-black/80 backdrop-blur-sm"
          style={{ zIndex: 100000 }}
          onClick={() => setSelectedSeventvBadge(null)}
        >
          <div 
            className="bg-secondary border border-borderSubtle rounded-xl shadow-2xl p-6 max-w-md w-full mx-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex justify-between items-start mb-4">
              <h3 className="text-xl font-bold text-textPrimary">{selectedSeventvBadge.name}</h3>
              <button
                onClick={() => setSelectedSeventvBadge(null)}
                className="p-1 hover:bg-white/5 rounded-lg transition-colors"
              >
                <X size={18} className="text-textSecondary" />
              </button>
            </div>
            
            {/* Large badge preview */}
            <div className="flex justify-center mb-6">
              <div className={`w-32 h-32 flex items-center justify-center bg-transparent rounded-xl ${isAnimatedBadge(selectedSeventvBadge) ? 'ring-4 ring-[#29b6f6]/50' : ''}`}>
                <img
                  src={getSeventvBadgeImageUrl(selectedSeventvBadge)}
                  alt={selectedSeventvBadge.name}
                  className="w-28 h-28 object-contain"
                />
              </div>
            </div>
            
            {/* Badge info */}
            <div className="space-y-3">
              {selectedSeventvBadge.description && (
                <div>
                  <span className="text-xs text-textSecondary uppercase tracking-wider">Description</span>
                  <p className="text-textPrimary mt-1">{selectedSeventvBadge.description}</p>
                </div>
              )}
              
              {/* Date Added */}
              <div>
                <span className="text-xs text-textSecondary uppercase tracking-wider">Added</span>
                <p className="text-textPrimary mt-1">{getFormattedCreationDate(selectedSeventvBadge.id)}</p>
              </div>
              
              {isAnimatedBadge(selectedSeventvBadge) && (
                <div className="flex items-center gap-2 text-[#29b6f6]">
                  <span className="w-2 h-2 bg-[#29b6f6] rounded-full animate-pulse"></span>
                  <span className="text-sm">Animated Badge</span>
                </div>
              )}
            </div>
          </div>
        </div>,
        document.body
      )}

      {/* 7TV Paint Detail Modal */}
      {selectedSeventvPaint && createPortal(
        <div 
          className="fixed inset-0 flex items-center justify-center bg-black/80 backdrop-blur-sm"
          style={{ zIndex: 100000 }}
          onClick={() => setSelectedSeventvPaint(null)}
        >
          <div 
            className="bg-secondary border border-borderSubtle rounded-xl shadow-2xl p-6 max-w-md w-full mx-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex justify-between items-start mb-4">
              <h3 className="text-xl font-bold text-textPrimary">{selectedSeventvPaint.name}</h3>
              <button
                onClick={() => setSelectedSeventvPaint(null)}
                className="p-1 hover:bg-white/5 rounded-lg transition-colors"
              >
                <X size={18} className="text-textSecondary" />
              </button>
            </div>
            
            {/* Large paint preview with user's name */}
            <div className="flex justify-center mb-6">
              <div className="px-8 py-6 bg-transparent rounded-xl">
                <span 
                  className="text-3xl font-bold"
                  style={{ 
                    background: generatePaintGradient(selectedSeventvPaint, 2),
                    backgroundClip: 'text',
                    WebkitBackgroundClip: 'text',
                    WebkitTextFillColor: 'transparent',
                    filter: generatePaintShadow(selectedSeventvPaint)
                  }}
                >
                  {currentUser?.display_name || currentUser?.login || 'YourName'}
                </span>
              </div>
            </div>
            
            {/* Paint gradient preview bar */}
            <div 
              className="h-8 rounded-lg mb-6"
              style={{ background: generatePaintGradient(selectedSeventvPaint, 2) }}
            />
            
            {/* Paint info */}
            <div className="space-y-3">
              {selectedSeventvPaint.description && (
                <div>
                  <span className="text-xs text-textSecondary uppercase tracking-wider">Description</span>
                  <p className="text-textPrimary mt-1">{selectedSeventvPaint.description}</p>
                </div>
              )}
              
              {/* Date Added */}
              <div>
                <span className="text-xs text-textSecondary uppercase tracking-wider">Added</span>
                <p className="text-textPrimary mt-1">{getFormattedCreationDate(selectedSeventvPaint.id)}</p>
              </div>

              {/* Usage */}
              {paintUsage.has(selectedSeventvPaint.id) && (
                <div>
                  <span className="text-xs text-textSecondary uppercase tracking-wider">Usage</span>
                  <p className="text-textPrimary mt-1">
                    Worn by {paintUsage.get(selectedSeventvPaint.id)!.toLocaleString()} users
                  </p>
                </div>
              )}

              {selectedSeventvPaint.tags.length > 0 && (
                <div>
                  <span className="text-xs text-textSecondary uppercase tracking-wider">Tags</span>
                  <div className="flex flex-wrap gap-2 mt-1">
                    {selectedSeventvPaint.tags.map((tag, i) => (
                      <span key={i} className="px-2 py-1 bg-[#29b6f6]/10 text-[#29b6f6] text-xs rounded-lg">
                        {tag}
                      </span>
                    ))}
                  </div>
                </div>
              )}

            </div>
          </div>
        </div>,
        document.body
      )}

      {/* Cosmetic Detail Modal */}
      {selectedCosmetic && createPortal(
        (() => {
          const cosmetic = selectedCosmetic;
          const asset = resolveCosmeticAsset(cosmetic);
          const owned = ownedCosmeticSlugs.has(cosmetic.slug);
          const isActive = activeCosmeticSlug === cosmetic.slug;
          // Switch only — never unequip. A StreamNook member always wears a
          // badge so their StreamNook identity stays visible to other members;
          // tapping the already-active badge does nothing rather than clearing
          // the selection back to none.
          const handleEquip = async () => {
            if (!currentUser?.user_id || isActive) return;
            await setActiveCosmetic(currentUser.user_id, cosmetic.slug);
          };
          // streamnook.app is the only purchase path. ko_fi_url is intentionally
          // ignored — Ko-fi is retired. Fall back to the generic support page if
          // a catalog row somehow has no stripe_url.
          const supportUrl = cosmetic.stripe_url || 'https://streamnook.app/support';
          const handleGet = async () => {
            if (!supportUrl) return;
            try {
              const { open } = await import('@tauri-apps/plugin-shell');
              const userLogin = currentUser?.login || currentUser?.username;
              const handle = userLogin ? `${supportUrl.includes('?') ? '&' : '?'}handle=${encodeURIComponent(userLogin)}` : '';
              await open(supportUrl + handle);
            } catch (err) {
              Logger.error('Failed to open support URL:', err);
            }
          };
          const acquireLabel = cosmetic.is_default
            ? 'Free for every StreamNook member'
            : cosmetic.payment_type === 'Subscription'
              ? 'Awarded for an active monthly subscription'
              : cosmetic.payment_type === 'Donation'
                ? 'Awarded for a one-time donation'
                : null;
          return (
            <div
              className="fixed inset-0 flex items-center justify-center bg-black/80 backdrop-blur-sm"
              style={{ zIndex: 100000 }}
              onClick={() => setSelectedCosmetic(null)}
            >
              <div
                className="bg-secondary border border-borderSubtle rounded-xl shadow-2xl p-6 max-w-md w-full mx-4"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="flex justify-between items-start mb-4">
                  <h3 className="text-xl font-bold text-textPrimary">{cosmetic.name}</h3>
                  <button
                    onClick={() => setSelectedCosmetic(null)}
                    className="p-1 hover:bg-white/5 rounded-lg transition-colors"
                  >
                    <X size={18} className="text-textSecondary" />
                  </button>
                </div>

                <div className="flex justify-center mb-6">
                  <div className="w-32 h-32 flex items-center justify-center bg-transparent rounded-xl">
                    {asset ? (
                      <img
                        src={asset}
                        alt={cosmetic.name}
                        className="w-28 h-28 object-contain"
                        draggable={false}
                      />
                    ) : (
                      <div className="w-28 h-28 rounded bg-white/[0.04]" />
                    )}
                  </div>
                </div>

                <div className="space-y-3">
                  {cosmetic.description && (
                    <div>
                      <span className="text-xs text-textSecondary uppercase tracking-wider">Description</span>
                      <p className="text-textPrimary mt-1">{cosmetic.description}</p>
                    </div>
                  )}

                  {acquireLabel && (
                    <div>
                      <span className="text-xs text-textSecondary uppercase tracking-wider">How to acquire</span>
                      <p className="text-textPrimary mt-1">{acquireLabel}</p>
                    </div>
                  )}

                  {cosmetic.animated && (
                    <div className="flex items-center gap-2 text-accent">
                      <span className="w-2 h-2 bg-accent rounded-full animate-pulse"></span>
                      <span className="text-sm">Animated Badge</span>
                    </div>
                  )}

                  <div className="flex gap-2 pt-2">
                    {owned ? (
                      <button
                        onClick={handleEquip}
                        disabled={!currentUser?.user_id}
                        className={`flex-1 px-4 py-2 text-sm font-medium rounded-lg transition-all ${
                          isActive
                            ? 'glass-input border border-accent/50 text-textPrimary cursor-default'
                            : 'glass-button text-textPrimary hover:bg-white/5'
                        }`}
                      >
                        {isActive ? 'Equipped' : 'Equip'}
                      </button>
                    ) : supportUrl ? (
                      <button
                        onClick={handleGet}
                        className="flex-1 px-4 py-2 text-sm font-medium glass-button text-textPrimary hover:bg-white/5 transition-all flex items-center justify-center gap-2"
                      >
                        Support StreamNook
                        <ExternalLink size={14} className="opacity-60" />
                      </button>
                    ) : null}
                  </div>
                </div>
              </div>
            </div>
          );
        })(),
        document.body
      )}
    </motion.div>
  );
};

export default BadgesOverlay;
