// Live drop progress for the stream you are watching.
//
// Where the numbers come from, because it is not the obvious place: the live
// `drop-progress` / `drops-progress-update` events are emitted by the desktop
// automation controller (a plugin component), not by Rust, so on mobile they
// never fire. `get_drop_progress` is not the answer either — it returns a
// zero-filled entry per drop in this build, which would render a bar stuck at
// 0%. The real accrued minutes ride along on each inventory drop's own embedded
// `progress`, which is what the desktop controller prefers for the same reason.
//
// So this polls the inventory and matches it to the category being watched.
import React, { useCallback, useEffect, useState } from 'react';
import { Gift } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { campaignEarnableOn } from '../dropsEligibility';
import { Logger } from '../../utils/logger';
import { channelHasEarnableCampaign, useDropsGameNames } from '../dropsCampaigns';
import type { InventoryResponse, TimeBasedDrop } from '../../types';
import { isBackgrounded } from '../backgroundGate';

// Inventory is a network round trip, and accrued minutes only ever move once a
// minute, so there is nothing a faster poll could learn.
//
// It also only runs on a channel that can actually earn something. Most watching
// is on channels with no campaign at all, and polling there asked the same
// question forever and was always going to get the same answer. That is the
// difference between a request a minute for the whole session and none.
const POLL_MS = 60_000;

interface Shown {
  dropId: string;
  /** Twitch wants this alongside the drop id to claim; absent on some entries. */
  dropInstanceId?: string;
  name: string;
  image?: string;
  /** Campaign this reward belongs to, e.g. "Ignite Stage 1 PlayOffs". */
  campaign: string | null;
  /** Used to jump straight to this campaign in Activity when the bar is tapped. */
  campaignId: string | null;
  /** 1-based tier position and total, e.g. 2 of 5. */
  tierIndex: number | null;
  tierCount: number | null;
  current: number;
  required: number;
}

interface Props {
  /** Lets the chat header know whether it has anything to show. The header is
   *  otherwise gated on hype train / pinned message and would never mount this. */
  onActiveChange?: (active: boolean) => void;
  /** False while a different chat tab is on screen. Progress belongs to the
   *  stream, not to whatever room you are reading, so it hides — but stays
   *  mounted so the poll keeps running and is current when you switch back. */
  visible?: boolean;
}

export const DropProgressBar: React.FC<Props> = ({ onActiveChange, visible = true }) => {
  const currentStream = useAppStore((s) => s.currentStream);
  const originCategory = useAppStore((s) => s.streamOriginCategory);
  const addToast = useAppStore((s) => s.addToast);
  const openDropCampaign = useMobileNavStore((s) => s.openDropCampaign);
  const [shown, setShown] = useState<Shown | null>(null);
  const [claiming, setClaiming] = useState(false);

  // `currentStream.game_name` is '' whenever startStream fell back to the
  // channel-info path, so the category the stream was opened from is a real
  // second source rather than a nicety.
  const gameName = currentStream?.game_name || originCategory?.name || '';
  const channelLogin = currentStream?.user_login;

  const dropsByGame = useDropsGameNames();
  const earnable = channelHasEarnableCampaign(dropsByGame, gameName, channelLogin);

  const refresh = useCallback(async () => {
    if (!gameName && !channelLogin) {
      setShown(null);
      return;
    }
    try {
      const connected = await invoke<boolean>('is_drops_authenticated').catch(() => false);
      if (!connected) {
        setShown(null);
        return;
      }
      const inv = await invoke<InventoryResponse>('get_drops_inventory').catch(() => null);
      if (!inv) return;

      // Only campaigns that this stream can actually advance. Watching Rust does
      // not earn a Fortnite drop, and showing one would imply it does.
      //
      // Category and allow-list are BOTH requirements, not alternatives.
      //
      // This has now been wrong in both directions, so the rule is worth stating
      // plainly: Twitch credits watch-time only while the channel is streaming
      // the campaign's game, AND, if the campaign names channels, only on those
      // channels. Either condition failing means zero progress.
      //
      // The first version treated an allow-list hit as sufficient, and showed a
      // Marvel Rivals drop advancing while the channel played something else.
      // The fix over-corrected into treating the category as sufficient, which
      // is this bug: watching any stream of the right game showed a bar for a
      // campaign restricted to a handful of channels the viewer was not on, so
      // it promised progress that could never arrive.
      //
      // A campaign flagged ACL that parses to zero channels is excluded, which
      // is deliberate: the backend calls that state unfarmable, so there is no
      // channel that could advance it and nothing honest to show.
      const target = gameName.toLowerCase();
      const login = (channelLogin || '').toLowerCase();
      const relevant = inv.items.filter((it) => {
        const campaign = it.campaign;
        // Shared with the stream card's drops icon, so the two cannot drift
        // apart again and promise different things about the same channel.
        if (!campaignEarnableOn(campaign, login)) return false;

        if (target) return (campaign.game_name || '').toLowerCase() === target;
        // Category genuinely unknown. Being on a campaign's allow-list is the
        // only evidence left, and an unrestricted campaign gives none.
        return (campaign.allowed_channels || []).length > 0 || campaign.is_acl_based;
      });
      const drops: TimeBasedDrop[] = relevant.flatMap((it) => it.campaign.time_based_drops || []);
      // Warn, not debug: debug is silenced by default, and a stream that Twitch
      // flags as drops-enabled but that we cannot match to a campaign is the one
      // case worth seeing in logcat without a devtools session.
      if (relevant.length === 0 || drops.length === 0) {
        Logger.warn(
          `[DropProgress] no match: game="${gameName}" channel="${login}" campaigns=${inv.items.length} relevant=${relevant.length} drops=${drops.length}`,
        );
      }

      // The tier being earned right now is the unclaimed collectible one with
      // the fewest minutes left, matching the backend's own choice rule.
      const candidates = drops
        .filter((d) => d.required_minutes_watched > 0 && !d.progress?.is_claimed)
        .map((d) => ({
          d,
          current: Math.min(d.progress?.current_minutes_watched ?? 0, d.required_minutes_watched),
          required: d.required_minutes_watched,
        }))
        .filter((c) => c.current < c.required);

      if (candidates.length === 0) {
        Logger.warn(
          `[DropProgress] matched ${drops.length} drop(s) but none collectible+unclaimed+incomplete`,
        );
        setShown(null);
        return;
      }
      candidates.sort((a, b) => a.required - a.current - (b.required - b.current));
      const best = candidates[0];

      // Which tier of its campaign this is. Campaigns ship several rewards at
      // increasing watch times, and "2 of 5" tells you far more about how far
      // through you are than the single bar does.
      const owner = relevant.find((it) =>
        (it.campaign.time_based_drops || []).some((d) => d.id === best.d.id),
      );
      const tiers = [...(owner?.campaign.time_based_drops || [])]
        .filter((d) => d.required_minutes_watched > 0)
        .sort((a, b) => a.required_minutes_watched - b.required_minutes_watched);
      const tierIndex = tiers.findIndex((d) => d.id === best.d.id);

      setShown({
        dropId: best.d.id,
        dropInstanceId: best.d.progress?.drop_instance_id,
        name: best.d.benefit_edges?.[0]?.name || best.d.name || 'Drop',
        image: best.d.benefit_edges?.[0]?.image_url,
        campaign: owner?.campaign.name ?? null,
        campaignId: owner?.campaign.id ?? null,
        tierIndex: tierIndex >= 0 ? tierIndex + 1 : null,
        tierCount: tiers.length || null,
        current: best.current,
        required: best.required,
      });
    } catch (err) {
      Logger.warn('[DropProgress] inventory read failed:', err);
    }
  }, [gameName, channelLogin]);

  // Nothing happens at all until this channel is known to have a campaign you
  // can earn on. The same test the stream card's drops icon uses, off the same
  // once-per-session campaign list, so the two cannot disagree.
  //
  // A campaign only enters the inventory ONCE YOU HAVE PROGRESS, so for the
  // first minute or two of watching an earnable channel there is genuinely
  // nothing to find and the bar stays empty. That wait is the drop accruing, not
  // a stall, and polling harder cannot shorten it.
  useEffect(() => {
    if (!earnable) return;
    void refresh();
    // Skipped while backgrounded rather than torn down: nobody is looking at
    // the bar, and it re-reads on the next tick after resuming anyway.
    const t = setInterval(() => {
      if (isBackgrounded()) return;
      void refresh();
    }, POLL_MS);
    return () => clearInterval(t);
  }, [refresh, earnable]);

  // Derived rather than cleared from an effect, so moving to a channel with no
  // campaign cannot leave the previous channel's bar on screen for a frame.
  const live = earnable ? shown : null;

  useEffect(() => {
    // Report inactive while hidden too, so the header does not reserve space for
    // a bar that is not being drawn.
    onActiveChange?.(visible && !!live);
    return () => onActiveChange?.(false);
  }, [live, visible, onActiveChange]);

  if (!live || !visible) return null;

  const pct = Math.min(100, (live.current / live.required) * 100);
  const complete = live.current >= live.required;
  const remaining = Math.max(0, live.required - live.current);

  const claim = async () => {
    if (claiming) return;
    setClaiming(true);
    try {
      await invoke('claim_drop', {
        dropId: live.dropId,
        dropInstanceId: live.dropInstanceId ?? null,
      });
      addToast('Drop claimed', 'success');
      await refresh();
    } catch (err) {
      Logger.error('[DropProgress] claim failed:', err);
      addToast('Could not claim that drop.', 'error');
    } finally {
      setClaiming(false);
    }
  };

  return (
    // Tapping the bar shrinks the player to the mini window rather than closing
    // it — the whole point is that watching (and therefore earning) continues
    // while you go look at the campaign.
    <div
      role="button"
      tabIndex={0}
      onClick={() => live.campaignId && openDropCampaign(live.campaignId)}
      // Carries its own container, like PinnedBanner does. The chat header that
      // used to supply a shared background is pure layout now, so anything
      // sitting in it has to be a self-contained floating card or it reads as
      // loose text over chat.
      className="sn-popover pointer-events-auto flex items-center gap-2.5 px-2.5 py-1.5 active:opacity-70"
    >
      {live.image ? (
        <img
          src={live.image}
          alt=""
          className="w-9 h-9 rounded-md object-cover shrink-0 ring-1 ring-white/10"
          draggable={false}
        />
      ) : (
        <div className="w-9 h-9 rounded-md bg-surface flex items-center justify-center shrink-0">
          <Gift size={16} className="text-accent" />
        </div>
      )}
      <div className="flex-1 min-w-0">
        {/* Line 1: what you are earning, and how long is left. */}
        <div className="flex items-baseline gap-2">
          <span className="text-[12px] font-semibold text-textPrimary truncate">{live.name}</span>
          <span
            className={`ml-auto text-[11px] shrink-0 tabular-nums ${
              complete ? 'text-success font-semibold' : 'text-textMuted'
            }`}
          >
            {complete ? 'Ready to claim' : `${remaining}m left`}
          </span>
        </div>
        {/* Line 2: which campaign, which tier, and the exact minutes. The bar
            alone says roughly-how-far; this says where you actually are. */}
        <div className="flex items-baseline gap-1.5 text-[10.5px] text-textMuted">
          {live.campaign && <span className="truncate min-w-0">{live.campaign}</span>}
          {live.tierIndex && live.tierCount && live.tierCount > 1 && (
            <span className="shrink-0">
              {live.campaign ? '· ' : ''}
              {live.tierIndex} of {live.tierCount}
            </span>
          )}
          {/* No minute count here. "36m left" on the line above already says
              where you are, and the bar shows the ratio, so printing 24/60m as
              well was the same fact three times. */}
        </div>
        <div className="h-1 rounded-full bg-surface overflow-hidden mt-1">
          <div
            className={`h-full rounded-full transition-[width] duration-700 ${
              complete ? 'bg-success' : 'bg-accent'
            }`}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      {complete && (
        <button
          onClick={() => void claim()}
          disabled={claiming}
          className="shrink-0 px-2.5 py-1 rounded-full text-[11px] font-semibold bg-success/20 text-success disabled:opacity-60"
        >
          {claiming ? '…' : 'Claim'}
        </button>
      )}
    </div>
  );
};
