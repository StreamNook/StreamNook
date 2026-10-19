// Live drop progress for the stream you are watching.
//
// Which drop, how far along and whether it waits on a claim are decided in
// Rust (`get_watch_drop`, services/watch_drop.rs), shared with the iPhone
// app: the campaign has to count in this category AND on this channel, and
// the inventory, a network read, is only asked when one does. The progress is
// the inventory's own per-drop minutes; `get_drop_progress` and the
// `drop-progress` events belong to the desktop automation and are empty here.
//
// It is one card of the chat's top stack (components/chat/chatTopStack.ts): a
// one-line strip (art, name, time left and a thin progress line) whose tap
// opens it, and an open form whose art and campaign line jump to Rewards.
import React, { useCallback, useEffect, useState } from 'react';
import { CaretRight, Gift } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { Logger } from '../../utils/logger';
import { isBackgrounded } from '../backgroundGate';
import { isTwitchStream } from '../../utils/streamProvider';
import { BannerStatus, OverlayBanner, StackCardPresence } from '../../components/chat/OverlayBanner';
import { useStackPresence, type StackCardControl } from '../../components/chat/chatTopStack';

// Accrued minutes only move once a minute, so a faster poll learns nothing.
const POLL_MS = 60_000;

export interface WatchDrop {
  campaign_id: string;
  drop_id: string;
  instance_id: string | null;
  name: string;
  image: string;
  /** "12m to watch"; empty once the minutes are in (the Claim button says it). */
  trailing: string;
  /** The strip's status: "12m to watch", or "Ready" once the minutes are in. */
  status: string;
  /** "Campaign · 2 of 5". */
  line: string;
  percent: number;
  complete: boolean;
  claim_label: string;
  spoken: string;
}

interface Props {
  /** Its slot in the chat's top stack. */
  stack: StackCardControl;
  /** False while a different chat tab is on screen. Progress belongs to the
   *  stream, not to whatever room you are reading, so it hides but stays
   *  mounted so the poll keeps running. */
  visible?: boolean;
}

export const DropProgressBar: React.FC<Props> = ({ stack, visible = true }) => {
  const currentStream = useAppStore((s) => s.currentStream);
  const originCategory = useAppStore((s) => s.streamOriginCategory);
  const addToast = useAppStore((s) => s.addToast);
  const openDropCampaign = useMobileNavStore((s) => s.openDropCampaign);
  const [shown, setShown] = useState<{ login: string; drop: WatchDrop } | null>(null);
  const [claiming, setClaiming] = useState(false);

  // `game_name` is '' whenever startStream fell back to the channel-info path,
  // so the category the stream was opened from is a real second source.
  const gameName = currentStream?.game_name || originCategory?.name || '';
  const gameId = currentStream?.game_id || '';
  const channelLogin = currentStream?.user_login ?? '';
  const channelId = currentStream?.user_id ?? '';
  // Drops are Twitch's; a Kick stream in a drops category earns nothing.
  const twitch = isTwitchStream(currentStream) && !!channelLogin;

  const refresh = useCallback(async () => {
    try {
      const drop = await invoke<WatchDrop | null>('get_watch_drop', {
        channelId,
        login: channelLogin,
        gameId,
        gameName,
      });
      setShown(drop ? { login: channelLogin, drop } : null);
    } catch (err) {
      Logger.warn('[DropProgress] read failed:', err);
    }
  }, [channelId, channelLogin, gameId, gameName]);

  useEffect(() => {
    if (!twitch) return;
    void refresh();
    // Skipped while backgrounded rather than torn down: nobody is looking at
    // the bar.
    const t = setInterval(() => {
      if (isBackgrounded()) return;
      void refresh();
    }, POLL_MS);
    // Coming back to the app reads at once. Minutes kept accruing while it was
    // away (audio keeps playing), and the next tick can be most of a minute
    // off, long enough to show a stale count or miss a drop now ready.
    const onVisible = () => {
      if (document.visibilityState === 'visible') void refresh();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(t);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [refresh, twitch]);

  // Derived rather than cleared from an effect, so moving to another channel
  // cannot leave the previous channel's bar on screen for a frame.
  const live = twitch && shown?.login === channelLogin ? shown.drop : null;
  const showing = visible && !!live;

  // The campaign is the card's identity: the next drop of the same campaign
  // keeps the viewer's fold, a new campaign starts fresh. Hidden counts as
  // gone, so the stack never keeps a slot for a card it is not drawing.
  // The card fades out when the drop ends and keeps its slot until it has.
  const released = useStackPresence(stack, showing && live ? live.campaign_id : null);

  const claim = async (e: React.MouseEvent) => {
    // A claim stays on the player; only a tap on the card's body jumps to Rewards.
    e.stopPropagation();
    if (!live || claiming) return;
    setClaiming(true);
    try {
      await invoke('claim_drop', { dropId: live.drop_id, dropInstanceId: live.instance_id });
      addToast('Drop claimed', 'success');
      await refresh();
    } catch (err) {
      Logger.error('[DropProgress] claim failed:', err);
      addToast('Could not claim that drop.', 'error');
    } finally {
      setClaiming(false);
    }
  };

  if (!showing || !live || !stack.placed) return <StackCardPresence onLeft={released} />;
  return (
    <StackCardPresence onLeft={released}>
    <DropCard
      key="drop"
      live={live}
      open={stack.open}
      order={stack.order}
      onToggle={stack.onToggle}
      claiming={claiming}
      onClaim={(e) => void claim(e)}
      onOpen={() => openDropCampaign(live.campaign_id)}
    />
    </StackCardPresence>
  );
};

const DropArt: React.FC<{ live: WatchDrop; size: 'strip' | 'open' }> = ({ live, size }) => {
  const box = size === 'strip' ? 'w-5 h-5 rounded' : 'w-9 h-9 rounded-md';
  return live.image ? (
    <img src={live.image} alt="" className={`${box} object-cover shrink-0 ring-1 ring-white/10`} draggable={false} />
  ) : (
    <span className={`${box} bg-textPrimary/[0.08] flex items-center justify-center shrink-0`}>
      <Gift size={size === 'strip' ? 12 : 16} className="text-accent-on-surface" />
    </span>
  );
};

export const DropCard: React.FC<{
  live: WatchDrop;
  open: boolean;
  order: number;
  onToggle: () => void;
  claiming: boolean;
  onClaim: (e: React.MouseEvent) => void;
  onOpen: () => void;
}> = ({ live, open, order, onToggle, claiming, onClaim, onOpen }) => {
  const fill = live.complete ? 'bg-success' : 'bg-accent-on-surface';
  return (
    <OverlayBanner
      icon={<DropArt live={live} size="strip" />}
      kind="Drop"
      title={live.name}
      badges={
        <BannerStatus mono={false} tone={live.complete ? 'done' : 'quiet'}>
          {live.status}
        </BannerStatus>
      }
      openIcon={<Gift size={16} className="text-accent-on-surface" />}
      isExpanded={open}
      onToggleExpanded={onToggle}
      order={order}
      stripLine={
        <span aria-hidden className="absolute left-0 right-0 bottom-0 h-[2px] bg-textPrimary/[0.08]">
          <span className={`block h-full ${fill} transition-[width] duration-700`} style={{ width: `${live.percent}%` }} />
        </span>
      }
    >
      {/* Tapping the body shrinks the player to the mini window rather than
          closing it: watching (and so earning) continues while you look at
          the campaign. The head above only folds the card. */}
      <div className="px-3 pb-3">
        <div
          role="button"
          tabIndex={0}
          aria-label={live.spoken}
          onClick={onOpen}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') onOpen();
          }}
          className="flex items-center gap-2.5 rounded-md active:opacity-70"
        >
          <DropArt live={live} size="open" />
          <div className="flex-1 min-w-0">
            <div className="text-[12px] leading-4 text-textMuted truncate">{live.line}</div>
            <div className="mt-1.5 h-1 rounded-full bg-textPrimary/[0.12] overflow-hidden">
              <div
                className={`h-full rounded-full ${fill} transition-[width] duration-700`}
                style={{ width: `${live.percent}%` }}
              />
            </div>
          </div>
          {live.complete ? (
            <button
              onClick={onClaim}
              disabled={claiming}
              className="shrink-0 px-2.5 py-1 rounded-full text-[12px] font-semibold border border-success/60 text-success disabled:opacity-60"
            >
              {claiming ? '...' : live.claim_label}
            </button>
          ) : (
            <CaretRight size={14} className="shrink-0 text-textMuted" />
          )}
        </div>
      </div>
    </OverlayBanner>
  );
};
