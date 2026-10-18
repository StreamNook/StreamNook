// Rewards: drops and badges, the two things you earn by watching.
//
// Named Activity until it was pointed out that the word describes neither of
// the things on it. Both tabs are collections of rewards, so the tab says that.
//
// Both segments are built in Rust and shared with the iPhone app: the Drops
// segment (`get_rewards_drops`, services/rewards_drops.rs: grouping, order,
// search, every row's words) and the badge wall (`get_badge_wall`,
// services/badge_wall.rs). This file lays them out. Connecting drops uses the
// redirect grant in the in-app login overlay; Twitch closed the device-code
// grant on the drops client.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { ArrowSquareOut, Broadcast, CalendarBlank, CaretRight, CheckCircle, Gift, Warning } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SearchField } from '../ui/SearchField';
import { MobileSheet } from '../ui/MobileSheet';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { badgeGalleryIsWarm, followBadgeSync, loadBadgeGallery, useBadgeGallery } from '../rewards/badgeGalleryStore';
import { MobileMissingNow } from '../rewards/MobileMissingNow';
import { BadgeSortMenu } from '../rewards/BadgeSortMenu';
import { ScreenHeader } from '../ui/ScreenHeader';
import { TitleSegments } from '../ui/TitleControls';
import { ProviderMark } from '../../components/ProviderLogo';
import {
  blurbText,
  getRewardsDrops,
  type BadgeSort,
  type BadgeTile,
  type CampaignCard,
  type RewardsDrops,
} from '../rewards/rewardsModel';
import { getBadgeEarnGuide, type EarnGuide } from '../../services/badgeStanding';
import { openExternal } from '../../utils/openExternal';
import { Logger } from '../../utils/logger';
import type { TwitchStream } from '../../types';

type RewardsTab = 'drops' | 'badges';
const REWARDS_TABS: { value: RewardsTab; label: string }[] = [
  { value: 'drops', label: 'Drops' },
  { value: 'badges', label: 'Badges' },
];

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
  countLabel: string;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}> = ({ art, game, countLabel, expanded, onToggle, children }) => (
  <div>
    <button
      onClick={onToggle}
      aria-expanded={expanded}
      className="w-full flex items-center gap-2 mt-3 mb-1.5 text-left active:opacity-70 transition-opacity"
    >
      {art ? (
        <img
          src={art}
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
        <div className="text-[12px] text-textMuted">{countLabel}</div>
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

const wordColor = (tile: BadgeTile) =>
  tile.owned
    ? 'text-accent'
    : tile.status === 'available'
      ? 'text-success'
      : tile.status === 'coming-soon'
        ? 'text-warning'
        : 'text-textMuted';

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
  // Center does.
  const [campaignDetail, setCampaignDetail] = useState<CampaignCard | null>(null);
  const [dropsQuery, setDropsQuery] = useState('');
  // Participating-channels list inside the campaign sheet. Stamped with the
  // campaign it was opened for and compared during render, so opening a
  // different campaign starts closed without an effect to reset it.
  const [channelsOpenFor, setChannelsOpenFor] = useState<string | null>(null);
  const startStream = useAppStore((s) => s.startStream);
  const wall = useBadgeGallery((s) => s.wall);
  const badgesLoading = useBadgeGallery((s) => s.loading);
  const syncLeft = useBadgeGallery((s) => s.syncLeft);
  // Whether the wall was already in memory when this screen mounted: a warm
  // wall snaps in rather than replaying the settle over every tile.
  const [warmOnMount] = useState(() => badgeGalleryIsWarm());
  const [badgeSort, setBadgeSort] = useState<BadgeSort>('newest');
  const [badgeDetail, setBadgeDetail] = useState<BadgeTile | null>(null);
  // How the open badge is earned and where, from Rust (badge_earn.rs).
  const [badgeGuide, setBadgeGuide] = useState<{ key: string; guide: EarnGuide } | null>(null);
  const [drops, setDrops] = useState<RewardsDrops | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  // The search as typed, read when an answer lands. Written from the input's
  // change handler rather than during render.
  const latestQuery = useRef('');
  const changeQuery = (value: string) => {
    latestQuery.current = value;
    setDropsQuery(value);
  };

  useEffect(() => {
    if (!badgeDetail) return;
    const [setId, versionId] = (badgeDetail.keys[0] ?? '').split('/');
    if (!setId || !versionId) return;
    let live = true;
    const key = badgeDetail.key;
    getBadgeEarnGuide(setId, versionId)
      .then((guide) => { if (live) setBadgeGuide({ key, guide }); })
      .catch(() => {});
    return () => { live = false; };
  }, [badgeDetail]);
  const openGuide = badgeDetail && badgeGuide?.key === badgeDetail.key ? badgeGuide.guide : null;

  // A badge's channel, opened the way the desktop badge page opens it: with the
  // live stream when there is one, so the player starts with its metadata.
  const watchBadgeChannel = async (login: string) => {
    const live = await invoke<TwitchStream | null>('check_stream_online', { userLogin: login }).catch(
      () => null,
    );
    setBadgeDetail(null);
    void startStream(login, live ?? undefined);
  };

  // A read on mount and on pull-to-refresh asks Twitch again; a new search
  // re-filters Rust's last read.
  //
  // Answers can land out of order (a refresh reads the network, a filter only
  // the cache), so one is drawn only if it was filtered by the search on
  // screen now; Rust echoes the trimmed query it used. A fresh read that lost
  // that race is filtered again by the current search, so its new data is
  // not thrown away.
  const load = useCallback(async (query: string, refresh: boolean) => {
    const current = () => latestQuery.current.trim();
    try {
      const next = await getRewardsDrops(query, refresh);
      if (next.query === current()) {
        setDrops(next);
        return;
      }
      if (!refresh) return;
      const refiltered = await getRewardsDrops(latestQuery.current, false);
      if (refiltered.query === current()) setDrops(refiltered);
    } catch (err) {
      Logger.warn('[Rewards] load failed:', err);
      if (query.trim() !== current()) return;
      setDrops({
        connected: false,
        query: '',
        in_progress: [],
        available: [],
        earned: [],
        in_progress_empty: '',
        no_match: '',
      });
    }
  }, []);

  useEffect(() => {
    void load('', true);
  }, [load]);

  useEffect(() => {
    if (!drops?.connected) return;
    const timer = setTimeout(() => void load(dropsQuery, false), 200);
    return () => clearTimeout(timer);
    // Re-filters on a new search only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dropsQuery]);

  // A jump from the drop bar opens the campaign's group, then scrolls to it
  // once the card has rendered.
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());
  const focusGroup = focusDropCampaignId
    ? [...(drops?.in_progress ?? []), ...(drops?.available ?? [])].find((g) =>
        g.campaigns.some((c) => c.id === focusDropCampaignId),
      )?.key ?? null
    : null;
  useEffect(() => {
    if (!focusDropCampaignId) return;
    if (tab !== 'drops') setTab('drops');
    const el = campaignRefs.current[focusDropCampaignId];
    if (!el) return;
    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    setFlashedCampaign(focusDropCampaignId);
    // The group stays open once the jump is used up.
    if (focusGroup) setOpenGroups((prev) => new Set(prev).add(focusGroup));
    clearDropFocus();
    const t = setTimeout(() => setFlashedCampaign(null), 1800);
    return () => clearTimeout(t);
  }, [focusDropCampaignId, focusGroup, drops, tab, clearDropFocus]);

  // The account is the signed-in main one, which Rust resolves itself; the id
  // only re-runs the load when the account changes.
  const uid = currentUser?.user_id;
  // Which account the open Badges segment last asked Twitch about; cleared on
  // leaving it, so every visit asks again.
  const badgeVisit = useRef<string | null>(null);
  useEffect(() => {
    if (tab !== 'badges') {
      badgeVisit.current = null;
      return;
    }
    followBadgeSync();
    // Opening the segment (or switching account) is when someone checks what
    // they have: Rust re-reads the collection in the background even when the
    // wall is still warm. A new sort only reorders what Rust already holds,
    // so it reads the cache.
    const visit = uid ?? '';
    const fresh = badgeVisit.current !== visit;
    badgeVisit.current = visit;
    void loadBadgeGallery(badgeSort, fresh);
  }, [tab, uid, badgeSort]);

  // A relay push, a claim, a finished collection read, the metadata backfill
  // or an earn window opening or closing: Rust says so, and the wall is read
  // again, but only once it has been read at all; the segment's own first
  // read covers a wall never opened.
  useEffect(() => {
    const unlisten = listen('badge-standing-changed', () => {
      if (useBadgeGallery.getState().wall) void loadBadgeGallery(badgeSort, false);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [badgeSort]);

  // The redirect grant, the same one desktop moved to when Twitch closed the
  // device-code grant on the drops client. The authorize page opens in the
  // in-app login WebView, Twitch redirects back with the credential, the
  // native overlay keeps it and closes itself, and Rust collects and stores
  // it. The page never sees the token.
  const connect = async () => {
    setConnecting(true);
    setConnectError(null);
    try {
      const url = await invoke<string>('start_drops_login');
      const landed = new Promise<boolean>((resolve) => {
        const done = (ok: boolean) => {
          window.removeEventListener('sn:drops-redirect', onLanded);
          window.removeEventListener('sn:login-cancelled', onCancel);
          clearTimeout(deadline);
          resolve(ok);
        };
        const onLanded = () => done(true);
        const onCancel = () => done(false);
        // Long enough to sign in from scratch if the session had lapsed.
        const deadline = setTimeout(() => done(false), 5 * 60 * 1000);
        window.addEventListener('sn:drops-redirect', onLanded);
        window.addEventListener('sn:login-cancelled', onCancel);
      });
      await invoke('open_mobile_login', { url, title: 'Connect drops' });
      if (!(await landed)) {
        setConnectError('Sign-in was closed before Twitch approved drops.');
        return;
      }
      await invoke('finish_mobile_drops_login');
      addToast('Drops connected!', 'success');
      await load(dropsQuery, true);
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      Logger.error('[Rewards] drops connect failed:', err);
      setConnectError(reason);
      addToast(`Drops connection failed: ${reason}`, 'error');
    } finally {
      await invoke('close_mobile_login').catch(() => {});
      setConnecting(false);
    }
  };

  const openMissing = (key: string) => {
    const tile = wall?.tiles.find((t) => t.keys.includes(key));
    if (tile) setBadgeDetail(tile);
  };

  const q = dropsQuery.trim();
  const toggleGroup = (key: string) =>
    setOpenGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  // A search opens everything it matched.
  const isGroupOpen = (key: string) => !!q || openGroups.has(key) || key === focusGroup;

  const inProgressGroups = drops?.in_progress ?? [];
  const availableGroups = drops?.available ?? [];
  const tiles = wall?.tiles ?? [];

  const dropsSettled = useSettleIn(
    drops?.connected === true && inProgressGroups.length + availableGroups.length > 0,
    `drops:${q}`,
  );
  const badgesSettled = useSettleIn(tiles.length > 0, `badges:${badgeSort}`, warmOnMount);

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      {/* The same header as Following and Browse, so the title sits on one
          baseline on every tab. Drops and Badges sit beside it (Brandon: in the
          title row, so the row under it goes and the content moves up). */}
      <ScreenHeader title="Rewards">
        <TitleSegments<RewardsTab> label="Rewards" segments={REWARDS_TABS} value={tab} onChange={setTab} />
      </ScreenHeader>
      <PullToRefresh onRefresh={tab === 'drops' ? () => load(dropsQuery, true) :() => loadBadgeGallery(badgeSort, true)}>
        <div className={`px-4 sn-tabbar-clearance ${tab === 'badges' ? '' : 'hidden'}`}>
          {tiles.length === 0 ? (
            <div className="py-10 text-center text-sm text-textMuted">
              {badgesLoading || !wall || wall.loading ? 'Loading badges…' : 'No badges available right now.'}
            </div>
          ) : (
            <>
              {/* What can still be earned leads; the whole wall follows under
                  its own heading. */}
              {wall && (
                <div className="pt-2">
                  <MobileMissingNow missing={wall.missing} onOpen={openMissing} />
                </div>
              )}
              <div className="flex items-center gap-2 pb-3">
                <div className="flex-1 min-w-0">
                  <h2 className="text-[12px] font-semibold text-textMuted uppercase tracking-wide">All badges</h2>
                  <div className="mt-[3px] flex items-center gap-1.5 text-[12px] text-textMuted min-w-0">
                    <span className="truncate">{wall?.count_label}</span>
                    {syncLeft > 0 && (
                      // The metadata backfill, in the strip's own "checking" language.
                      <span className="flex items-center gap-1.5 shrink-0">
                        <span aria-hidden className="missing-now-pulse" />
                        <span>Updating badges</span>
                      </span>
                    )}
                  </div>
                </div>
                <BadgeSortMenu sort={badgeSort} onChange={setBadgeSort} />
              </div>
              <div className="grid grid-cols-3 sm:grid-cols-5 lg:grid-cols-7 gap-2">
                {tiles.map((tile, bi) => {
                  const lit = tile.owned || tile.status === 'available';
                  return (
                    <SettleIn key={tile.key} index={bi} settled={badgesSettled} className="sn-badge-tile">
                      <button
                        onClick={() => setBadgeDetail(tile)}
                        className={`glass-panel p-2 flex flex-col items-center gap-1.5 relative active:opacity-80 w-full h-full ${
                          tile.status === 'available' && !tile.owned ? 'ring-1 ring-success/70' : ''
                        } ${tile.owned ? 'ring-1 ring-accent/60' : ''}`}
                      >
                        <img
                          src={tile.image}
                          alt=""
                          loading="lazy"
                          decoding="async"
                          className={`w-11 h-11 object-contain ${lit ? '' : 'opacity-45 grayscale'}`}
                          draggable={false}
                        />
                        {/* Two lines reserved whether the name needs them or
                            not, so the status word lines up across a row. */}
                        <span
                          className={`w-full min-h-[28px] text-[11px] text-center line-clamp-2 leading-[14px] [overflow-wrap:anywhere] ${
                            lit ? 'text-textPrimary' : 'text-textMuted'
                          }`}
                        >
                          {tile.title}
                        </span>
                        <span className={`text-[11px] font-semibold leading-none ${wordColor(tile)}`}>{tile.word}</span>
                      </button>
                    </SettleIn>
                  );
                })}
              </div>
            </>
          )}
        </div>
        <div className={`px-4 sn-tabbar-clearance ${tab === 'drops' ? '' : 'hidden'}`}>
          {drops?.connected === false && (
            <div className="glass-panel p-4 mt-2">
              <div className="flex items-center gap-2 mb-1.5">
                <ProviderMark provider="twitch" size={15} />
                <span className="text-[15px] font-semibold text-textPrimary">Twitch Drops</span>
              </div>
              <p className="text-[13px] text-textSecondary mb-3 leading-relaxed">
                Connect drops to track campaign progress and earn while you watch.
              </p>
              <button
                onClick={() => void connect()}
                disabled={connecting}
                className="glass-button sn-touch w-full text-[14px] font-semibold text-textPrimary disabled:opacity-60 flex items-center justify-center gap-1.5"
              >
                {connecting ? 'Approve on Twitch…' : 'Connect drops'}
                {!connecting && <ArrowSquareOut size={15} />}
              </button>
              {connectError && (
                <div className="mt-2 text-[12px] text-error leading-snug break-words">{connectError}</div>
              )}
            </div>
          )}

          {drops?.connected && (
            <>
              {/* Searching by game, since that is how anyone thinks about
                  drops. Campaign names are matched too. */}
              <SearchField
                value={dropsQuery}
                onChange={changeQuery}
                placeholder="Search games or campaigns"
                className="mb-1"
              />

              {drops.no_match ? (
                // A search that found nothing anywhere says so once, rather
                // than as three empty sections.
                <p className="mt-4 text-[12.5px] text-textMuted leading-snug">{drops.no_match}</p>
              ) : (
                <>
                  <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mt-2 mb-1.5">
                    In progress
                  </div>
                  {inProgressGroups.length === 0 ? (
                    <div className="glass-panel p-4 text-[13px] text-textMuted">{drops.in_progress_empty}</div>
                  ) : (
                    inProgressGroups.map((group, gIdx) => (
                      <SettleIn key={group.key} index={gIdx} settled={dropsSettled}>
                        <GameGroupSection
                          art={group.art}
                          game={group.game}
                          countLabel={group.count_label}
                          expanded={isGroupOpen(group.key)}
                          onToggle={() => toggleGroup(group.key)}
                        >
                          {group.campaigns.map((card) => (
                            <div
                              key={card.id}
                              ref={(el) => {
                                campaignRefs.current[card.id] = el;
                              }}
                              role="button"
                              tabIndex={0}
                              onClick={() => setCampaignDetail(card)}
                              className={`glass-panel p-2.5 flex gap-2.5 transition-shadow duration-500 active:opacity-80 ${
                                flashedCampaign === card.id ? 'ring-2 ring-accent' : ''
                              }`}
                            >
                              <div className="flex-1 min-w-0 flex flex-col justify-center">
                                <div className="flex items-center justify-between gap-2">
                                  <span className="text-[13.5px] font-medium text-textPrimary truncate">{card.name}</span>
                                  <span className="text-[12px] text-textMuted shrink-0">{card.claimed_label}</span>
                                </div>
                                <div className="h-1.5 rounded-full bg-surface overflow-hidden mt-1.5">
                                  <div
                                    className="h-full rounded-full bg-accent transition-[width]"
                                    style={{ width: `${card.percent}%` }}
                                  />
                                </div>
                              </div>
                            </div>
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
                        <SettleIn key={group.key} index={inProgressGroups.length + gIdx} settled={dropsSettled}>
                          <GameGroupSection
                            art={group.art}
                            game={group.game}
                            countLabel={group.count_label}
                            expanded={isGroupOpen(group.key)}
                            onToggle={() => toggleGroup(group.key)}
                          >
                            {group.campaigns.map((card) => (
                              <div
                                key={card.id}
                                ref={(el) => {
                                  campaignRefs.current[card.id] = el;
                                }}
                                role="button"
                                tabIndex={0}
                                onClick={() => setCampaignDetail(card)}
                                className="glass-panel p-2.5 flex gap-2.5 active:opacity-80"
                              >
                                {card.art ? (
                                  <img
                                    src={card.art}
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
                                    <span className="text-[13.5px] font-medium text-textPrimary truncate">{card.name}</span>
                                    {card.needs_link && <Warning size={14} className="text-warning shrink-0" />}
                                  </div>
                                  <div className="text-[11.5px] text-textMuted mt-1">{card.detail}</div>
                                </div>
                              </div>
                            ))}
                          </GameGroupSection>
                        </SettleIn>
                      ))}
                    </>
                  )}

                  {drops.earned.length > 0 && (
                    <>
                      <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mt-4 mb-1.5">
                        Recently earned
                      </div>
                      <div className="flex flex-col gap-2">
                        {drops.earned.map((drop) => (
                          <div key={drop.id} className="glass-panel p-2.5 flex items-center gap-2.5">
                            {drop.image ? (
                              <img src={drop.image} alt="" className="w-9 h-9 rounded object-cover shrink-0" draggable={false} />
                            ) : (
                              <Gift size={20} className="text-accent shrink-0" />
                            )}
                            <div className="flex-1 min-w-0">
                              <div className="text-[13.5px] text-textPrimary truncate">{drop.name}</div>
                              {drop.game && <div className="text-[12px] text-textMuted truncate">{drop.game}</div>}
                            </div>
                            <CheckCircle size={16} className="text-success shrink-0" />
                          </div>
                        ))}
                      </div>
                    </>
                  )}
                </>
              )}
            </>
          )}
        </div>
      </PullToRefresh>

      {/* Badge detail: art, status, how it is earned and when. */}
      <MobileSheet open={!!badgeDetail} onClose={() => setBadgeDetail(null)} maxHeightFraction={0.8}>
        {badgeDetail && (
          <div className="flex flex-col items-center text-center pb-1">
            <img src={badgeDetail.image} alt="" className="w-20 h-20 object-contain mb-2.5" draggable={false} />
            <div className="text-[16px] font-semibold text-textPrimary">{badgeDetail.title}</div>
            <div className="mt-1 mb-2.5">
              {badgeDetail.sheet_word && (
                <span className={`text-[11px] font-semibold ${wordColor(badgeDetail)}`}>{badgeDetail.sheet_word}</span>
              )}
            </div>
            {(() => {
              const prose = blurbText(badgeDetail.prose);
              const window = blurbText(badgeDetail.window);
              const summary = openGuide?.summary ?? null;
              const channels = (openGuide?.earn.channels ?? []).slice(0, 10);
              return (
                <>
                  {/* The campaign's own numbers in one sentence: what, how
                      long, and where. The writeup below it is colour. */}
                  {summary && (
                    <p className="text-[14px] font-medium text-textPrimary leading-relaxed text-left w-full">
                      {summary}
                    </p>
                  )}
                  {prose && prose.trim() !== summary && (
                    <p
                      className={`text-[13px] text-textSecondary leading-relaxed whitespace-pre-line text-left w-full${summary ? ' mt-2' : ''}`}
                    >
                      {prose}
                    </p>
                  )}
                  {/* The only channels it counts on, each a tap away. */}
                  {channels.length > 0 && (
                    <div className="mt-3 w-full flex flex-col gap-1.5">
                      {channels.map((login) => (
                        <button
                          key={login}
                          onClick={() => void watchBadgeChannel(login)}
                          className="glass-panel sn-touch px-3 py-2.5 flex items-center gap-2.5 text-left active:opacity-80"
                        >
                          <Broadcast size={16} className="text-accent shrink-0" />
                          <span className="flex-1 min-w-0 truncate text-[13.5px] font-medium text-textPrimary">
                            {login}
                          </span>
                          <span className="text-[12px] text-textMuted shrink-0">Watch</span>
                          <CaretRight size={13} weight="bold" className="text-textMuted shrink-0" />
                        </button>
                      ))}
                    </div>
                  )}
                  {badgeDetail.caveat && (
                    <div className="mt-3 w-full flex items-start gap-2 rounded-lg px-3 py-2 bg-amber-500/10 border border-amber-500/25">
                      <Warning size={14} weight="fill" className="text-amber-400 shrink-0 mt-px" />
                      <span className="text-[12.5px] text-amber-300/90 leading-snug text-left">{badgeDetail.caveat}</span>
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
            {badgeDetail.usage_label && (
              <p className="text-[12px] text-textMuted mt-3 self-center">{badgeDetail.usage_label}</p>
            )}
            {badgeDetail.info_url && (
              <button
                onClick={() => {
                  void invoke('open_browser_url', { url: badgeDetail.info_url }).catch(() => {});
                }}
                className="glass-button sn-touch mt-4 w-full text-[13.5px] font-semibold text-textPrimary"
              >
                More info
              </button>
            )}
          </div>
        )}
      </MobileSheet>

      {/* Every reward in one campaign, in the order you earn them. Sized to
          its content up to 80 percent; "Find a stream" stays at the bottom. */}
      <MobileSheet open={!!campaignDetail} onClose={() => setCampaignDetail(null)} maxHeightFraction={0.8}>
        {campaignDetail && (
          <div className="flex flex-col gap-2">
            {/* Title and its one line of context, centered as one block. The
                sheet's own title slot is a 14 px label in the drag handle, too
                slight to head a campaign. */}
            <div className="flex flex-col items-center gap-1 text-center pt-2 pb-2">
              <h2 className="text-[16px] font-semibold text-textPrimary leading-snug">{campaignDetail.name}</h2>
              <div className="text-[12.5px] text-textMuted tabular-nums">
                {[campaignDetail.game_name, campaignDetail.meta_label].filter(Boolean).join(' · ')}
              </div>
            </div>

            {/* Twitch will not credit watch time until the game account is
                linked, so saying this up front is the difference between a
                wasted evening and a working one. */}
            {campaignDetail.needs_link && (
              <button
                onClick={() => {
                  const url = campaignDetail.link_url;
                  if (!url) return;
                  void openExternal(url).then((ok) => {
                    if (!ok) addToast('Could not open the account link page.', 'error');
                  });
                }}
                disabled={!campaignDetail.link_url}
                className="glass-panel p-2.5 flex items-center gap-2 text-left disabled:opacity-70"
              >
                <Warning size={16} className="text-warning shrink-0" />
                <span className="flex-1 text-[12.5px] text-textSecondary leading-snug">
                  Your game account is not linked, so this campaign will not earn yet.
                </span>
                {campaignDetail.link_url && <ArrowSquareOut size={14} className="text-textMuted shrink-0" />}
              </button>
            )}

            {/* Where it can be earned: collapsed, each name a row to tap and watch. */}
            {campaignDetail.channels.length > 0 &&
              (() => {
                const channelsOpen = channelsOpenFor === campaignDetail.id;
                return (
                  <div>
                    <button
                      onClick={() => setChannelsOpenFor(channelsOpen ? null : campaignDetail.id)}
                      aria-expanded={channelsOpen}
                      className="w-full glass-panel px-3 py-2.5 flex items-center gap-2 text-left active:opacity-80"
                    >
                      <span className="text-[12.5px] text-textSecondary flex-1">{campaignDetail.channels_label}</span>
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
                          {/* About 40 percent of the screen, so eighty channels
                              never push the rewards away. */}
                          <div className="glass-panel max-h-[40vh] overflow-y-auto mt-1 flex flex-col">
                            {campaignDetail.channels.map((c) => (
                              <button
                                key={c.id || c.name}
                                onClick={() => {
                                  setCampaignDetail(null);
                                  setChannelsOpenFor(null);
                                  // The broadcaster id rides along so drops
                                  // monitoring targets this channel even when it
                                  // is not followed; startStream backfills the rest.
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
                                aria-label={`Watch ${c.name}`}
                                className="shrink-0 h-10 px-3 flex items-center gap-2 text-left border-b border-borderSubtle last:border-b-0 active:opacity-70"
                              >
                                <span className="flex-1 min-w-0 truncate text-[13px] text-textPrimary">{c.name}</span>
                                {/* Says what the tap does: it leaves the sheet
                                    and starts that channel. */}
                                <span className="shrink-0 text-[12px] text-textMuted">Watch</span>
                              </button>
                            ))}
                          </div>
                        </motion.div>
                      )}
                    </AnimatePresence>
                  </div>
                );
              })()}

            {campaignDetail.drops.map((drop) => (
              <div key={drop.id} className="flex items-center gap-2.5 py-1">
                {drop.image ? (
                  <img
                    src={drop.image}
                    alt=""
                    draggable={false}
                    className={`w-10 h-10 rounded-md object-cover shrink-0 ring-1 ring-white/10 ${
                      drop.claimed ? '' : 'opacity-90'
                    }`}
                  />
                ) : (
                  <div className="w-10 h-10 rounded-md bg-surface flex items-center justify-center shrink-0">
                    <Gift size={16} className="text-textMuted" />
                  </div>
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-baseline gap-2">
                    <span className="text-[13px] text-textPrimary truncate">{drop.name}</span>
                    <span
                      className={`ml-auto text-[11px] shrink-0 tabular-nums ${
                        drop.claimed ? 'text-success' : drop.done ? 'text-success font-semibold' : 'text-textMuted'
                      }`}
                    >
                      {drop.status}
                    </span>
                  </div>
                  {drop.detail && <div className="text-[11px] text-textMuted truncate mt-0.5">{drop.detail}</div>}
                  {/* A drop watch time does not earn gets no bar to imply otherwise. */}
                  {drop.bar != null && (
                    <div className="h-1 rounded-full bg-surface overflow-hidden mt-1.5">
                      <div
                        className={`h-full rounded-full ${drop.done ? 'bg-success' : 'bg-accent'}`}
                        style={{ width: `${drop.bar}%` }}
                      />
                    </div>
                  )}
                </div>
              </div>
            ))}

            {/* Last, and sticky: a long campaign scrolls its rewards under the
                button instead of pushing the way to watch out of reach. The
                band behind it fades the rows out rather than cutting them.
                glass-exempt: a fade-out band in the sheet's own color, solid where the button sits. */}
            {campaignDetail.find_stream && (
              <div
                className="sticky bottom-0 -mx-4 px-4 pt-3 pb-1 mt-1"
                style={{
                  background: 'linear-gradient(to top, var(--color-background-tertiary) 65%, transparent)',
                }}
              >
                <button
                  onClick={() => {
                    const category = campaignDetail.find_stream!;
                    setCampaignDetail(null);
                    openBrowseCategory(category);
                    setNavTab('browse');
                  }}
                  className="glass-button sn-touch w-full rounded-lg text-[13.5px] font-semibold text-textPrimary"
                >
                  Find a stream
                </button>
              </div>
            )}
          </div>
        )}
      </MobileSheet>
    </div>
  );
};
