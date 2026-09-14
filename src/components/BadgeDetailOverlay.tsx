import { X, Gift, ArrowLeft, AlertTriangle, Calendar, ChevronRight, Users, Clock, ArrowUpRight } from 'lucide-react';
import { useEffect, useState, useMemo, useCallback, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import type { TwitchStream } from '../types';
import { parseBadgeForLinks, type ParsedBadgeLink } from '../services/badgeParsingService';
import { Tooltip } from './ui/Tooltip';
import { motion, AnimatePresence } from 'framer-motion';
import { BadgeChannelChip } from './badge/BadgeChannelChip';
import { BadgeChannelCard } from './badge/BadgeChannelCard';
import { BadgeCategoryCard } from './badge/BadgeCategoryCard';
import { BadgeSiblingChips } from './badge/BadgeSiblingChips';
import { BadgeEligibleCategories } from './badge/BadgeEligibleCategories';

import { Logger } from '../utils/logger';
import { decodeHtmlEntities, deriveBadgeStatus } from '../utils/badgeWindow';
import { extractChannelLogins, splitChannelMentions } from '../utils/badgeChannels';
import { dedupeResolvedLinks } from '../utils/badgeLinks';

// The sibling-list heading, from our enrichment or from badgebase. Everything
// after it (up to the next blank line) is the list of related event badges.
const SIBLING_MARKER = /(?:Also part of this event|Other badges related to this event):\s*/i;
const WINDOW_LINE = /(?:Event duration|Event time):\s*(.+)/i;

// Date-chip styling for an active vs upcoming earn window.
const CHIP_ACTIVE = 'bg-success/15 text-success ring-1 ring-success/30';
const CHIP_UPCOMING = 'bg-info/15 text-info ring-1 ring-info/30';

// Split a badge's more_info into its earn paragraph, the event window, and the
// sibling list, so each renders in its own place in the redesigned panel.
function parseBadgeMore(
  moreInfo: string,
  enrichmentRelated?: string | null,
): { earnProse: string; window: string | null; siblingText: string } {
  const idx = moreInfo.search(SIBLING_MARKER);
  const before = idx === -1 ? moreInfo : moreInfo.slice(0, idx);
  const afterRaw = idx === -1 ? '' : moreInfo.slice(idx).replace(SIBLING_MARKER, '');
  const siblingText = (enrichmentRelated || afterRaw).trim().split(/\n\n/)[0];
  const winMatch = before.match(WINDOW_LINE);
  const earnProse = before.replace(WINDOW_LINE, '').replace(/\n{3,}/g, '\n\n').trim();
  return { earnProse, window: winMatch ? winMatch[1].trim() : null, siblingText };
}

const MONTHS_LC = [
  'january', 'february', 'march', 'april', 'may', 'june',
  'july', 'august', 'september', 'october', 'november', 'december',
];

// Streamer logins a badge points at: "/studbudz" and natural-language "X's
// (Twitch) channel". Deduped, lowercased. Unresolvable names self-filter (the
// card renders nothing when the channel isn't found).
// Rewrite "Month D, YYYY at HH:MM UTC" to the viewer's local time so badge text
// never shows a raw UTC time.
function localizeUtcInText(text: string): string {
  const re = new RegExp(
    `\\b(${MONTHS_LC.join('|')})\\s+(\\d{1,2})(?:st|nd|rd|th)?,?\\s+(\\d{4})\\s+at\\s+(\\d{1,2}):(\\d{2})\\s*UTC\\b`,
    'gi',
  );
  return text.replace(re, (full, mon, day, year, hh, mm) => {
    const mi = MONTHS_LC.indexOf(String(mon).toLowerCase());
    if (mi < 0) return full;
    const d = new Date(Date.UTC(+year, mi, +day, +hh, +mm));
    if (Number.isNaN(d.getTime())) return full;
    return d.toLocaleString(undefined, {
      month: 'long',
      day: 'numeric',
      year: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    });
  });
}
interface TwitchCategory {
  id: string;
  name: string;
  box_art_url?: string;
}

interface DropCampaign {
  id: string;
  name: string;
  game_name: string;
  game_id: string;
}

interface InventoryItem {
  campaign: DropCampaign;
  status: string;
}

interface InventoryResponse {
  items: InventoryItem[];
}

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

interface EnrichmentData {
  how_to_earn?: string | null;
  action?: string | null;
  highlight?: string | null;
  caveats?: string | null;
  related?: string | null;
  footnote?: string | null;
  distribution?: string | null;
  /// Exact Twitch category from the badge's Drops campaign. Authoritative, so it
  /// beats guessing a name out of the prose.
  category?: string | null;
  /// Logins of the specific channels the badge names, already validated by the
  /// relay. Preferred over reading them back out of the prose.
  channels?: string[] | null;
  starts_utc?: string | null;
  ends_utc?: string | null;
}

interface BadgeMetadata {
  date_added: string | null;
  usage_stats: string | null;
  more_info: string | null;
  enrichment?: EnrichmentData | null;
  info_url: string;
}

// A link whose name came from the Drops campaign rather than from prose, so it
// is searched first and ahead of any name guessed out of the copy.
type LinkCandidate = ParsedBadgeLink & { authoritative?: boolean };

/**
 * Recover a real category from an over-specific name by dropping trailing words
 * ("La Velada del Año VI" -> "La Velada"). Only accepts a result the original
 * name starts with, so it narrows to a genuine parent rather than drifting to
 * an unrelated category.
 */
async function recoverCategoryByPrefix(name: string): Promise<TwitchCategory | null> {
  const words = name.trim().split(/\s+/);
  for (let len = words.length - 1; len >= 2; len--) {
    const query = words.slice(0, len).join(' ');
    try {
      const results = await invoke<TwitchCategory[]>('search_categories', { query, limit: 5 });
      const hit = (results ?? []).find(
        (r) => name.toLowerCase().startsWith(r.name.toLowerCase()) && r.name.length >= 4
      );
      if (hit) return hit;
    } catch {
      return null;
    }
  }
  return null;
}

// A refined badge link that also carries the category's box art for a cover chip
// and the id Twitch resolved it to, which is what identity is judged on.
type DisplayLink = ParsedBadgeLink & { boxArtUrl?: string; categoryId?: string };

interface BadgeDetailOverlayProps {
  badge: BadgeVersion;
  setId: string;
  onClose: () => void;
  onBack: () => void;
}

const BadgeDetailOverlay = ({ badge, setId, onClose, onBack }: BadgeDetailOverlayProps) => {
  const [badgeBaseInfo, setBadgeBaseInfo] = useState<BadgeMetadata | null>(null);
  const [loadingBadgeBase, setLoadingBadgeBase] = useState(true);
  const [techOpen, setTechOpen] = useState(false);
  const [refinedLinks, setRefinedLinks] = useState<DisplayLink[]>([]);
  const [loadingCategories, setLoadingCategories] = useState(false);

  // Navigation actions only, so no store subscription at all: read them once
  // (stable for the store's lifetime) instead of re-rendering on every write.
  const { navigateToCategoryByName, openDropsWithSearch, setShowBadgesOverlay, startStream } =
    useAppStore.getState();

  // Deep links for the badge. The campaign category is used verbatim when the
  // relay supplied one; the prose parser is a fallback for badges without a
  // campaign, and its heuristics reject names carrying punctuation or accents
  // (e.g. "MARVEL TOKON: Fighting Souls").
  const parsedLinks = useMemo(() => {
    const fromProse = parseBadgeForLinks(badge.description, badgeBaseInfo?.more_info ?? undefined);
    const campaignCategory = badgeBaseInfo?.enrichment?.category?.trim();
    if (!campaignCategory) return fromProse;
    return [
      {
        type: 'category' as const,
        name: campaignCategory,
        originalText: campaignCategory,
        authoritative: true,
      },
      ...fromProse.filter(
        (l) => l.type !== 'category' || l.name.toLowerCase() !== campaignCategory.toLowerCase()
      ),
    ] as LinkCandidate[];
  }, [badge.description, badgeBaseInfo?.more_info, badgeBaseInfo?.enrichment?.category]);

  // Search Twitch for better category/drops names and validate they exist
  const refineLinks = useCallback(async (links: LinkCandidate[]) => {
    if (links.length === 0) {
      setRefinedLinks([]);
      return;
    }

    setLoadingCategories(true);
    const validLinks: DisplayLink[] = [];

    for (const link of links) {
      if (link.type === 'category') {
        // Validate and refine category names using Twitch search API
        try {
          const results = await invoke<TwitchCategory[]>('search_categories', {
            query: link.name,
            limit: 5
          });

          if (results && results.length > 0) {
            // Find the best match - prefer exact match, then longest name that contains our search term
            const searchLower = link.name.toLowerCase();

            let bestMatch = results[0];

            // First, check for exact match (case-insensitive)
            const exactMatch = results.find(r => r.name.toLowerCase() === searchLower);
            if (exactMatch) {
              bestMatch = exactMatch;
            } else {
              // Otherwise, find the best fuzzy match
              // Prioritize results where our search term is contained in the result name
              const containingMatches = results.filter(r =>
                r.name.toLowerCase().includes(searchLower) ||
                searchLower.includes(r.name.toLowerCase())
              );

              if (containingMatches.length > 0) {
                // Prefer longer names as they're more specific (e.g., "Tom Clancy's Rainbow Six Siege X")
                bestMatch = containingMatches.reduce((best, curr) =>
                  curr.name.length > best.name.length ? curr : best
                );
              }
            }

            Logger.debug(`[BadgeDetail] Refined category: "${link.name}" → "${bestMatch.name}"`);

            // Add the validated link with refined name + box art for the chip.
            validLinks.push({
              ...link,
              name: bestMatch.name,
              boxArtUrl: bestMatch.box_art_url,
              categoryId: bestMatch.id,
            });
          } else {
            // Event names get mistaken for categories ("La Velada del Año VI"
            // when the category is "La Velada"), so retry on progressively
            // shorter prefixes and accept only a result the full name starts
            // with. A name Twitch cannot resolve is dropped rather than
            // rendered as a card that leads nowhere.
            const recovered = await recoverCategoryByPrefix(link.name);
            if (recovered) {
              Logger.debug(`[BadgeDetail] Recovered category "${link.name}" -> "${recovered.name}"`);
              validLinks.push({
                ...link,
                name: recovered.name,
                boxArtUrl: recovered.box_art_url,
                categoryId: recovered.id,
              });
            } else {
              Logger.debug(`[BadgeDetail] No matching category found for "${link.name}", skipping`);
            }
          }
        } catch (error) {
          Logger.warn(`[BadgeDetail] Failed to search categories for "${link.name}":`, error);
          // Keep original link on error
          validLinks.push(link);
        }
      } else if (link.type === 'drops') {
        // Validate drops event names using active drops campaigns
        try {
          const inventory = await invoke<InventoryResponse>('get_drops_inventory');

          if (inventory && inventory.items && inventory.items.length > 0) {
            const searchLower = link.name.toLowerCase();

            // Search for matching campaign name or game name
            const matchingCampaign = inventory.items.find(item => {
              const campaignLower = item.campaign.name.toLowerCase();
              const gameLower = item.campaign.game_name.toLowerCase();

              return campaignLower.includes(searchLower) ||
                searchLower.includes(campaignLower) ||
                gameLower.includes(searchLower) ||
                searchLower.includes(gameLower);
            });

            if (matchingCampaign) {
              Logger.debug(`[BadgeDetail] Refined drops: "${link.name}" → "${matchingCampaign.campaign.name}"`);

              // Add the validated link with official campaign name
              validLinks.push({
                ...link,
                name: matchingCampaign.campaign.name,
              });
            } else {
              Logger.debug(`[BadgeDetail] No matching drops campaign found for "${link.name}", skipping`);
              // Don't add invalid drops links
            }
          } else {
            Logger.debug(`[BadgeDetail] No drops campaigns available to validate "${link.name}", skipping`);
            // Don't add drops links if we can't validate them
          }
        } catch (error) {
          Logger.warn(`[BadgeDetail] Failed to search drops for "${link.name}":`, error);
          // Don't add unvalidated drops links
        }
      }
    }

    setRefinedLinks(dedupeResolvedLinks(validLinks));
    setLoadingCategories(false);
  }, []);

  // Refine and validate links when parsed links change
  useEffect(() => {
    if (parsedLinks.length > 0) {
      refineLinks(parsedLinks);
    } else {
      setRefinedLinks([]);
    }
  }, [parsedLinks, refineLinks]);

  // Use refined links if available, otherwise fall back to parsed links
  // Refined links are already deduped; the unrefined fallback is deduped here
  // so a card can never appear twice even before Twitch has answered.
  const displayLinks: DisplayLink[] =
    refinedLinks.length > 0 ? refinedLinks : dedupeResolvedLinks(parsedLinks);

  // Handle clicking on a parsed link
  const handleLinkClick = (link: ParsedBadgeLink) => {
    // Close the badge overlay first
    setShowBadgesOverlay(false);
    onClose();

    if (link.type === 'category') {
      navigateToCategoryByName(link.name);
    } else if (link.type === 'drops') {
      openDropsWithSearch(link.name);
    }
  };

  // Open a streamer's channel exactly like everywhere else in the app: close the
  // badge overlays first, then hand startStream the stream info so it loads with
  // all the usual services (player, chat, metadata) rather than a half-state.
  const handleWatchChannel = (login: string, streamInfo?: TwitchStream) => {
    setShowBadgesOverlay(false);
    onClose();
    void startStream(login, streamInfo);
  };

  // Fetch BadgeBase.co information
  useEffect(() => {
    const fetchBadgeBaseInfo = async () => {
      try {
        setLoadingBadgeBase(true);
        const info = await invoke<BadgeMetadata>('fetch_badge_metadata', {
          badgeSetId: setId,
          badgeVersion: badge.id,
        });
        setBadgeBaseInfo(info);
      } catch (error) {
        Logger.warn('[BadgeDetail] Failed to fetch BadgeBase info:', error);
        // Silently fail - BadgeBase info is optional
      } finally {
        setLoadingBadgeBase(false);
      }
    };

    fetchBadgeBaseInfo();
  }, [setId, badge.id]);

  // Live-amend: when the relay pushes enrichment for this badge, refresh the
  // panel in place (no reopen needed).
  useEffect(() => {
    const unlisten = listen<{ badge_set_id: string; badge_version: string }>(
      'badge-metadata-amended',
      (event) => {
        const p = event.payload;
        if (p.badge_set_id === setId && p.badge_version === badge.id) {
          invoke<BadgeMetadata>('fetch_badge_metadata', {
            badgeSetId: setId,
            badgeVersion: badge.id,
          })
            .then(setBadgeBaseInfo)
            .catch(() => {});
        }
      }
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setId, badge.id]);

  // Derived from the window rather than read off the payload, so a badge whose
  // earn period opens while the app is running stops reading "Coming Soon".
  const badgeStatus = deriveBadgeStatus(
    badgeBaseInfo?.more_info,
    badgeBaseInfo?.enrichment as Record<string, unknown> | undefined
  );
  const isAvailable = badgeStatus === 'available';
  const isComingSoon = badgeStatus === 'coming-soon';

  // Parsed once here so the hero (window), the body, and the footer can each use
  // the piece they need.
  const badgeEnrichment = badgeBaseInfo?.enrichment;
  const parsedBadge = badgeBaseInfo?.more_info
    ? parseBadgeMore(badgeBaseInfo.more_info, badgeEnrichment?.related)
    : null;
  const badgeCaveats = badgeEnrichment?.caveats?.trim();
  const isDrops = (badgeBaseInfo?.more_info || '').toLowerCase().includes('twitch drops');
  // Streamers this badge points at, as themed live cards.
  // The relay validates channel handles for badges it enriches, so those win.
  // Everything else is read out of the prose, which names a channel in several
  // shapes ("the participating channel StudBudz", "/studbudz", "Ibai's
  // channel"). Capped because a few event badges list dozens of participating
  // streamers, and a wall of cards buries the rest of the panel.
  // The copy the eligible-categories list is read out of. Both sources are
  // searched because the list lives in whichever one the enricher filled.
  const eligibleCategoryText = [badgeBaseInfo?.more_info, badge.description]
    .filter(Boolean)
    .join('\n');

  const relayChannels = badgeBaseInfo?.enrichment?.channels ?? [];
  const channelLogins = (
    relayChannels.length > 0
      ? relayChannels
      : extractChannelLogins(
          [badgeBaseInfo?.more_info, badgeBaseInfo?.enrichment?.action, badge.description]
            .filter(Boolean)
            .join('\n')
        )
  ).slice(0, 6);

  // Render More Info with channel mentions ("/studbudz") turned into clickable
  // avatar chips, keeping the date-highlighting on the surrounding text.
  const renderMoreInfo = (text: string) => {
    return splitChannelMentions(localizeUtcInText(text)).map((part, i) =>
      part.login ? (
        <BadgeChannelChip key={i} login={part.login} onWatch={handleWatchChannel} />
      ) : (
        <span key={i}>{convertTimestampsToLocalJSX(part.text)}</span>
      ),
    );
  };


  // Convert timestamps to local time and return as JSX with highlighted dates
  // Handles both ISO timestamps and abbreviated date ranges like "Dec 1-12"
  const convertTimestampsToLocalJSX = (inputText: string): JSX.Element => {
    // First decode HTML entities
    const text = decodeHtmlEntities(inputText);
    const months: Record<string, number> = {
      'Jan': 0, 'Feb': 1, 'Mar': 2, 'Apr': 3,
      'May': 4, 'Jun': 5, 'Jul': 6, 'Aug': 7,
      'Sep': 8, 'Oct': 9, 'Nov': 10, 'Dec': 11
    };
    const fullMonths: Record<string, number> = {
      'January': 0, 'February': 1, 'March': 2, 'April': 3,
      'May': 4, 'June': 5, 'July': 6, 'August': 7,
      'September': 8, 'October': 9, 'November': 10, 'December': 11
    };

    const currentYear = new Date().getFullYear();

    // Check for "Month D, YYYY – Month D, YYYY" format (e.g., "December 6, 2025 – December 7, 2025")
    const fullDateRangeMatch = text.match(/(\w+)\s+(\d{1,2}),?\s+(\d{4})\s*[–-]\s*(\w+)\s+(\d{1,2}),?\s+(\d{4})/);
    if (fullDateRangeMatch) {
      const startMonthName = fullDateRangeMatch[1];
      const startDay = parseInt(fullDateRangeMatch[2], 10);
      const startYear = parseInt(fullDateRangeMatch[3], 10);
      const endMonthName = fullDateRangeMatch[4];
      const endDay = parseInt(fullDateRangeMatch[5], 10);
      const endYear = parseInt(fullDateRangeMatch[6], 10);

      if (Object.hasOwn(fullMonths, startMonthName) && Object.hasOwn(fullMonths, endMonthName)) {
        const startDate = new Date(startYear, fullMonths[startMonthName], startDay, 0, 0, 0);
        const endDate = new Date(endYear, fullMonths[endMonthName], endDay, 23, 59, 59);

        // Format the dates
        const formattedStartDate = startDate.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric'
        });
        const formattedEndDate = endDate.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric'
        });

        // Determine styling based on badge status
        let startClassName = 'px-2 py-0.5 rounded font-medium inline-block ';
        let endClassName = 'px-2 py-0.5 rounded font-medium inline-block ';

        if (isAvailable) {
          startClassName += CHIP_ACTIVE;
          endClassName += CHIP_ACTIVE;
        } else if (isComingSoon) {
          startClassName += CHIP_UPCOMING;
          endClassName += CHIP_UPCOMING;
        } else {
          startClassName += 'bg-accent/20 text-accent';
          endClassName += 'bg-accent/20 text-accent';
        }

        // Replace the date range with formatted dates
        const beforeMatch = text.substring(0, fullDateRangeMatch.index);
        const afterMatch = text.substring(fullDateRangeMatch.index! + fullDateRangeMatch[0].length);

        return (
          <>
            {beforeMatch}
            <span className={startClassName}>{formattedStartDate}</span>
            {' – '}
            <span className={endClassName}>{formattedEndDate}</span>
            {afterMatch}
          </>
        );
      }
    }

    // Check for "Mon DD – Mon DD" format (e.g., "Dec 06 – Dec 07")
    const fullRangeMatch = text.match(/(\w{3})\s+(\d{1,2})\s*[–-]\s*(\w{3})\s+(\d{1,2})/);
    if (fullRangeMatch) {
      const startMonthAbbrev = fullRangeMatch[1];
      const startDay = parseInt(fullRangeMatch[2], 10);
      const endMonthAbbrev = fullRangeMatch[3];
      const endDay = parseInt(fullRangeMatch[4], 10);

      if (Object.hasOwn(months, startMonthAbbrev) && Object.hasOwn(months, endMonthAbbrev)) {
        const startMonthNum = months[startMonthAbbrev];
        const endMonthNum = months[endMonthAbbrev];
        const startDate = new Date(currentYear, startMonthNum, startDay, 0, 0, 0);
        const endDate = new Date(currentYear, endMonthNum, endDay, 23, 59, 59);

        // Format the dates
        const formattedStartDate = startDate.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric'
        });
        const formattedEndDate = endDate.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric'
        });

        // Determine styling based on badge status
        let startClassName = 'px-2 py-0.5 rounded font-medium inline-block ';
        let endClassName = 'px-2 py-0.5 rounded font-medium inline-block ';

        if (isAvailable) {
          startClassName += CHIP_ACTIVE;
          endClassName += CHIP_ACTIVE;
        } else if (isComingSoon) {
          startClassName += CHIP_UPCOMING;
          endClassName += CHIP_UPCOMING;
        } else {
          startClassName += 'bg-accent/20 text-accent';
          endClassName += 'bg-accent/20 text-accent';
        }

        // Replace the abbreviated date with formatted dates
        const beforeMatch = text.substring(0, fullRangeMatch.index);
        const afterMatch = text.substring(fullRangeMatch.index! + fullRangeMatch[0].length);

        return (
          <>
            {beforeMatch}
            <span className={startClassName}>{formattedStartDate}</span>
            {' – '}
            <span className={endClassName}>{formattedEndDate}</span>
            {afterMatch}
          </>
        );
      }
    }

    // Check for "Mon D-D" format (e.g., "Dec 1-12")
    const shortRangeMatch = text.match(/(\w{3})\s+(\d{1,2})\s*[–-]\s*(\d{1,2})(?!\s*\w)/);
    if (shortRangeMatch) {
      const monthAbbrev = shortRangeMatch[1];
      const startDay = parseInt(shortRangeMatch[2], 10);
      const endDay = parseInt(shortRangeMatch[3], 10);

      if (Object.hasOwn(months, monthAbbrev)) {
        const monthNum = months[monthAbbrev];
        const startDate = new Date(currentYear, monthNum, startDay, 0, 0, 0);
        const endDate = new Date(currentYear, monthNum, endDay, 23, 59, 59);

        // Format the dates
        const formattedStartDate = startDate.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric'
        });
        const formattedEndDate = endDate.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric'
        });

        // Determine styling based on badge status
        let startClassName = 'px-2 py-0.5 rounded font-medium inline-block ';
        let endClassName = 'px-2 py-0.5 rounded font-medium inline-block ';

        if (isAvailable) {
          startClassName += CHIP_ACTIVE;
          endClassName += CHIP_ACTIVE;
        } else if (isComingSoon) {
          startClassName += CHIP_UPCOMING;
          endClassName += CHIP_UPCOMING;
        } else {
          startClassName += 'bg-accent/20 text-accent';
          endClassName += 'bg-accent/20 text-accent';
        }

        // Replace the abbreviated date with formatted dates
        const beforeMatch = text.substring(0, shortRangeMatch.index);
        const afterMatch = text.substring(shortRangeMatch.index! + shortRangeMatch[0].length);

        return (
          <>
            {beforeMatch}
            <span className={startClassName}>{formattedStartDate}</span>
            {' – '}
            <span className={endClassName}>{formattedEndDate}</span>
            {afterMatch}
          </>
        );
      }
    }

    // Fallback: Match ISO 8601 timestamps in the format: 2025-09-12T17:00 or 2025-09-12T17:00:00Z
    const isoRegex = /(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?(?:Z)?)/g;

    // Extract all timestamps first to determine start and end
    const timestamps = text.match(isoRegex);


    // Special handling for single timestamp
    if (timestamps && timestamps.length === 1) {
      const match = isoRegex.exec(text);
      if (match) {
        try {
          const startDate = new Date(match[0]);

          // Calculate end time based on duration
          let endDate: Date;
          const durationMatch = text.match(/(\d+)\s+(minute|hour)s?/i);
          if (durationMatch) {
            const duration = parseInt(durationMatch[1], 10);
            const unit = durationMatch[2].toLowerCase();
            endDate = new Date(startDate);
            if (unit === 'minute') {
              endDate.setMinutes(endDate.getMinutes() + duration);
            } else if (unit === 'hour') {
              endDate.setHours(endDate.getHours() + duration);
            }
          } else {
            // No duration found, assume event lasts until end of that day
            endDate = new Date(startDate.getFullYear(), startDate.getMonth(), startDate.getDate(), 23, 59, 59);
          }

          const formattedStartDate = startDate.toLocaleString(undefined, {
            year: 'numeric',
            month: 'long',
            day: 'numeric',
            hour: 'numeric',
            minute: '2-digit',
            hour12: true
          });

          const formattedEndDate = endDate.toLocaleString(undefined, {
            year: 'numeric',
            month: 'long',
            day: 'numeric',
            hour: 'numeric',
            minute: '2-digit',
            hour12: true
          });

          // Determine styling based on badge status
          let startClassName = 'px-2 py-0.5 rounded font-medium inline-block ';
          let endClassName = 'px-2 py-0.5 rounded font-medium inline-block ';

          if (isAvailable) {
            startClassName += CHIP_ACTIVE;
            endClassName += CHIP_ACTIVE;
          } else if (isComingSoon) {
            startClassName += CHIP_UPCOMING;
            endClassName += CHIP_UPCOMING;
          } else {
            startClassName += 'bg-accent/20 text-accent';
            endClassName += 'bg-accent/20 text-accent';
          }

          const beforeMatch = text.substring(0, match.index);
          const afterMatch = text.substring(match.index + match[0].length);

          return (
            <>
              {beforeMatch}
              <span className={startClassName}>{formattedStartDate}</span>
              {' – '}
              <span className={endClassName}>{formattedEndDate}</span>
              {afterMatch}
            </>
          );
        } catch {
          // If parsing fails, fall through to normal text handling
        }
      }
    }

    // Multiple timestamps handling
    const parts: (string | JSX.Element)[] = [];
    let lastIndex = 0;
    let match;
    let matchIndex = 0;

    isoRegex.lastIndex = 0; // Reset regex after previous exec

    while ((match = isoRegex.exec(text)) !== null) {
      // Add text before the match
      if (match.index > lastIndex) {
        parts.push(text.substring(lastIndex, match.index));
      }

      // Format and add the highlighted timestamp
      try {
        const date = new Date(match[0]);
        const formattedDate = date.toLocaleString(undefined, {
          year: 'numeric',
          month: 'long',
          day: 'numeric',
          hour: 'numeric',
          minute: '2-digit',
          hour12: true
        });

        // Determine if this is the start date (first timestamp) or end date (last timestamp)
        const isStartDate = timestamps && matchIndex === 0;
        const isEndDate = timestamps && matchIndex === timestamps.length - 1;

        // Determine styling based on badge status and which date this is
        let className = 'px-2 py-0.5 rounded font-medium inline-block ';

        if (isAvailable) {
          // Badge is available now - highlight the active period
          if (isStartDate && timestamps.length > 1) {
            // Start date - when it became available (green with glow)
            className += CHIP_ACTIVE;
          } else if (isEndDate && timestamps.length > 1) {
            // End date - when it expires (softer green)
            className += CHIP_ACTIVE;
          } else {
            // Other dates
            className += 'bg-accent/20 text-accent';
          }
        } else if (isComingSoon) {
          // Badge is coming soon - highlight the start date
          if (isStartDate && timestamps.length > 1) {
            // Start date - when it will become available (blue with glow)
            className += CHIP_UPCOMING;
          } else if (isEndDate && timestamps.length > 1) {
            // End date - when it will expire (softer blue)
            className += CHIP_UPCOMING;
          } else {
            // Other dates
            className += 'bg-accent/20 text-accent';
          }
        } else {
          // Badge is expired or no special status - use neutral accent color
          className += 'bg-accent/20 text-accent';
        }

        parts.push(
          <span key={match.index} className={className}>
            {formattedDate}
          </span>
        );
      } catch {
        // If parsing fails, add original text
        parts.push(match[0]);
      }

      lastIndex = match.index + match[0].length;
      matchIndex++;
    }

    // Add remaining text after last match
    if (lastIndex < text.length) {
      parts.push(text.substring(lastIndex));
    }

    return <>{parts}</>;
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.18 }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-2xl"
    >
      {/* Hover-sensitive background overlay */}
      <div
        className="absolute inset-0 group-hover:pointer-events-none"
        onClick={onClose}
      />

      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 12 }}
        transition={{ type: 'spring', stiffness: 380, damping: 30 }}
        className="sn-light-off liquid-glass-panel w-[90vw] max-h-[85vh] max-w-5xl flex flex-col relative z-10 overflow-hidden"
      >
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-borderSubtle">
          <div className="flex items-center gap-3">
            <Tooltip content="Back to badges" side="bottom">
              <button
                onClick={onBack}
                className="p-2 hover:bg-glass rounded-lg transition-all group"
              >
                <ArrowLeft size={20} className="text-textSecondary group-hover:text-textPrimary transition-colors" />
              </button>
            </Tooltip>
            <div>
              <div className="flex items-center gap-2">
                <h2 className="text-xl font-bold text-textPrimary">{badge.title}</h2>
                {isAvailable && (
                  <div className="flex items-center gap-1.5 px-2 py-1 bg-success/20 border border-success/50 rounded-full">
                    <span className="w-2 h-2 bg-success rounded-full animate-pulse"></span>
                    <span className="text-xs font-medium text-success">Available Now</span>
                  </div>
                )}
                {isComingSoon && (
                  <div className="flex items-center gap-1.5 px-2 py-1 bg-info/20 border border-info/50 rounded-full">
                    <span className="w-2 h-2 bg-info rounded-full animate-pulse"></span>
                    <span className="text-xs font-medium text-info">Coming Soon</span>
                  </div>
                )}
              </div>
              <p className="text-sm text-accent">Twitch Chat Badge</p>
            </div>
          </div>
          <Tooltip content="Close" side="bottom">
            <button
              onClick={onClose}
              className="p-2 hover:bg-glass rounded-lg transition-colors"
            >
              <X size={20} className="text-textSecondary" />
            </button>
          </Tooltip>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 custom-scrollbar">
          <div className="max-w-3xl mx-auto space-y-8">
            {/* Badge Variations */}
            <div className="flex items-end gap-4">
              <Tooltip content="View 72px image" side="top">
                <a
                  href={badge.image_url_4x}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center justify-center bg-glass rounded-lg p-4 hover:bg-glass/80 transition-colors cursor-pointer"
                >
                  <img
                    src={badge.image_url_4x}
                    alt={badge.title}
                    className="w-18 h-18 object-contain"
                  />
                </a>
              </Tooltip>
              <Tooltip content="View 36px image" side="top">
                <a
                  href={badge.image_url_2x}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center justify-center bg-glass rounded-lg p-3 hover:bg-glass/80 transition-colors cursor-pointer"
                >
                  <img
                    src={badge.image_url_2x}
                    alt={badge.title}
                    className="w-9 h-9 object-contain"
                  />
                </a>
              </Tooltip>
              <Tooltip content="View 18px image" side="top">
                <a
                  href={badge.image_url_1x}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center justify-center bg-glass rounded-lg p-2 hover:bg-glass/80 transition-colors cursor-pointer"
                >
                  <img
                    src={badge.image_url_1x}
                    alt={badge.title}
                    className="w-[18px] h-[18px] object-contain"
                  />
                </a>
              </Tooltip>
            </div>

            {/* When: the event window sits up top where you look first. */}
            {parsedBadge?.window && (
              <div className="flex items-center gap-1.5 text-[13px] text-textSecondary -mt-2">
                <Calendar size={14} className="shrink-0 text-textMuted" />
                {convertTimestampsToLocalJSX(parsedBadge.window)}
              </div>
            )}

            {/* Redesigned badge body: a typeset page, with the raw fields tucked
                into a collapsible. Keeps every field, presented richly. */}
            {/* Body: how to earn (the richer of enrichment prose / description,
                never both — they say the same thing at two lengths), then tiers. */}
            <div className="space-y-5">
              {(parsedBadge?.earnProse || badge.description) && (
                <div>
                  <h4 className="text-[12px] font-semibold text-textSecondary uppercase tracking-wide mb-2">
                    How to earn
                  </h4>
                  <div className="text-textPrimary text-[15px] leading-relaxed whitespace-pre-line">
                    {renderMoreInfo(parsedBadge?.earnProse || badge.description || '')}
                  </div>
                </div>
              )}

              {badgeCaveats && (
                <div className="flex gap-2.5 items-start bg-warning/10 rounded-lg px-3 py-2.5 text-[14px] text-warning/90">
                  <AlertTriangle size={15} className="text-warning mt-0.5 shrink-0" />
                  <span>{badgeCaveats}</span>
                </div>
              )}

              {/* Themed navigation: cover-art category card, streamer live cards,
                  and any drops-event card. */}
              {(displayLinks.length > 0 || channelLogins.length > 0) && (
                <div className="flex flex-col gap-2">
                  {displayLinks.map((link, index) =>
                    link.type === 'category' ? (
                      <BadgeCategoryCard
                        key={`link-${index}`}
                        name={link.name}
                        boxArtUrl={link.boxArtUrl}
                        onClick={() => handleLinkClick(link)}
                      />
                    ) : (
                      <button
                        key={`link-${index}`}
                        onClick={() => handleLinkClick(link)}
                        className="group flex items-center gap-3 w-full text-left p-2.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.06] transition-colors"
                      >
                        <span className="w-[46px] h-[46px] rounded-lg bg-white/[0.06] flex items-center justify-center shrink-0">
                          <Gift size={20} className="text-accent" />
                        </span>
                        <div className="flex-1 min-w-0">
                          <div className="text-[11px] text-textMuted uppercase tracking-wide">
                            Drops event
                          </div>
                          <div className="text-[15px] font-medium text-textPrimary truncate group-hover:text-accent transition-colors">
                            {link.name}
                          </div>
                          <div className="text-[12px] text-textSecondary">View the drops campaign</div>
                        </div>
                        <ArrowUpRight
                          size={18}
                          className="text-textMuted group-hover:text-accent transition-colors shrink-0"
                        />
                      </button>
                    ),
                  )}
                  {channelLogins.map((login) => (
                    <BadgeChannelCard key={`chan-${login}`} login={login} onWatch={handleWatchChannel} />
                  ))}
                  {loadingCategories && (
                    <div className="text-xs text-textMuted px-1">Finding category…</div>
                  )}
                </div>
              )}

              {/* Where it counts, when the copy spells the list out. */}
              <BadgeEligibleCategories
                text={eligibleCategoryText}
                onOpen={(name) => handleLinkClick({ type: 'category', name, originalText: name })}
              />

              {parsedBadge?.siblingText && (
                <div>
                  <h4 className="text-[12px] font-semibold text-textSecondary uppercase tracking-wide mb-2">
                    All tiers
                  </h4>
                  <BadgeSiblingChips related={parsedBadge.siblingText} currentTitle={badge.title} />
                </div>
              )}
            </div>

            {/* Quiet footer: stats + collapsible technical details */}
            <div className="space-y-3 pt-4 border-t border-white/[0.06]">
              {(badgeBaseInfo?.usage_stats || badgeBaseInfo?.date_added || isDrops) && (
                <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-[12px] text-textMuted">
                  {badgeBaseInfo?.usage_stats && (
                    <span className="inline-flex items-center gap-1.5">
                      <Users size={13} className="shrink-0" />
                      {badgeBaseInfo.usage_stats}
                    </span>
                  )}
                  {badgeBaseInfo?.date_added && (
                    <span className="inline-flex items-center gap-1.5">
                      <Clock size={13} className="shrink-0" />
                      Added {badgeBaseInfo.date_added}
                    </span>
                  )}
                  {isDrops && (
                    <span className="inline-flex items-center gap-1.5">
                      <Gift size={13} className="shrink-0" />
                      Twitch Drops
                    </span>
                  )}
                </div>
              )}
              <div>
                <button
                  type="button"
                  onClick={() => setTechOpen((o) => !o)}
                  className="inline-flex items-center gap-1 text-[12px] text-textMuted hover:text-textSecondary select-none"
                >
                  <ChevronRight
                    size={13}
                    className={`transition-transform duration-200 ${techOpen ? 'rotate-90' : ''}`}
                  />
                  Technical details
                </button>
                <AnimatePresence initial={false}>
                  {techOpen && (
                    <motion.div
                      key="tech"
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.25, ease: [0.4, 0, 0.2, 1] }}
                      className="overflow-hidden"
                    >
                      <div className="mt-2 flex flex-col gap-1.5 text-[13px]">
                  <div className="flex gap-3">
                    <span className="text-textSecondary w-24 shrink-0">ID</span>
                    <span className="text-textPrimary break-all">{setId}</span>
                  </div>
                  <div className="flex gap-3">
                    <span className="text-textSecondary w-24 shrink-0">Version</span>
                    <span className="text-textPrimary">{badge.id}</span>
                  </div>
                  <div className="flex gap-3">
                    <span className="text-textSecondary w-24 shrink-0">Title</span>
                    <span className="text-textPrimary">{badge.title}</span>
                  </div>
                  {badge.click_action && (
                    <div className="flex gap-3">
                      <span className="text-textSecondary w-24 shrink-0">Click action</span>
                      <span className="text-textPrimary">{badge.click_action}</span>
                    </div>
                  )}
                  {badge.click_url && (
                    <div className="flex gap-3">
                      <span className="text-textSecondary w-24 shrink-0">Click URL</span>
                      <a
                        href={badge.click_url}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-accent hover:underline break-all"
                      >
                        {badge.click_url}
                      </a>
                    </div>
                  )}
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            </div>

            {/* Loading state for BadgeBase info */}
            {loadingBadgeBase && (
              <div className="flex items-center justify-center py-4">
                <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-accent"></div>
                <span className="ml-3 text-textSecondary text-sm">Loading additional badge info...</span>
              </div>
            )}

          </div>
        </div>
      </motion.div>
    </motion.div>
  );
};

export default BadgeDetailOverlay;
