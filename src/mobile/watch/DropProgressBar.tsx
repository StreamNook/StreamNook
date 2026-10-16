// Live drop progress for the stream you are watching.
//
// Which drop, how far along and whether it waits on a claim are decided in
// Rust (`get_watch_drop`, services/watch_drop.rs), shared with the iPhone
// app: the campaign has to count in this category AND on this channel, and
// the inventory, a network read, is only asked when one does. The progress is
// the inventory's own per-drop minutes; `get_drop_progress` and the
// `drop-progress` events belong to the desktop automation and are empty here.
import React, { useCallback, useEffect, useState } from 'react';
import { Gift } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { Logger } from '../../utils/logger';
import { isBackgrounded } from '../backgroundGate';
import { isTwitchStream } from '../../utils/streamProvider';

// Accrued minutes only move once a minute, so a faster poll learns nothing.
const POLL_MS = 60_000;

interface WatchDrop {
  campaign_id: string;
  drop_id: string;
  instance_id: string | null;
  name: string;
  image: string;
  /** "12m left" or "Ready to claim". */
  trailing: string;
  /** "Campaign · 2 of 5". */
  line: string;
  percent: number;
  complete: boolean;
  claim_label: string;
  spoken: string;
}

interface Props {
  /** Lets the chat header know whether it has anything to show. */
  onActiveChange?: (active: boolean) => void;
  /** False while a different chat tab is on screen. Progress belongs to the
   *  stream, not to whatever room you are reading, so it hides but stays
   *  mounted so the poll keeps running. */
  visible?: boolean;
}

export const DropProgressBar: React.FC<Props> = ({ onActiveChange, visible = true }) => {
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
    // the bar, and it re-reads on the next tick after resuming anyway.
    const t = setInterval(() => {
      if (isBackgrounded()) return;
      void refresh();
    }, POLL_MS);
    return () => clearInterval(t);
  }, [refresh, twitch]);

  // Derived rather than cleared from an effect, so moving to another channel
  // cannot leave the previous channel's bar on screen for a frame.
  const live = twitch && shown?.login === channelLogin ? shown.drop : null;

  useEffect(() => {
    // Report inactive while hidden too, so the header does not reserve space
    // for a bar that is not being drawn.
    onActiveChange?.(visible && !!live);
    return () => onActiveChange?.(false);
  }, [live, visible, onActiveChange]);

  if (!live || !visible) return null;

  const claim = async (e: React.MouseEvent) => {
    // A claim stays on the player; only a tap on the card jumps to Rewards.
    e.stopPropagation();
    if (claiming) return;
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

  return (
    // Tapping the bar shrinks the player to the mini window rather than closing
    // it: watching (and so earning) continues while you look at the campaign.
    <div
      role="button"
      tabIndex={0}
      aria-label={live.spoken}
      onClick={() => openDropCampaign(live.campaign_id)}
      className="sn-popover sn-popover--static pointer-events-auto flex items-center gap-2.5 px-2.5 py-1.5 active:opacity-70"
    >
      {live.image ? (
        <img src={live.image} alt="" className="w-9 h-9 rounded-md object-cover shrink-0 ring-1 ring-white/10" draggable={false} />
      ) : (
        <div className="w-9 h-9 rounded-md bg-surface flex items-center justify-center shrink-0">
          <Gift size={16} className="text-accent" />
        </div>
      )}
      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-2">
          <span className="text-[12px] font-semibold text-textPrimary truncate">{live.name}</span>
          <span
            className={`ml-auto text-[11px] shrink-0 tabular-nums ${
              live.complete ? 'text-success font-semibold' : 'text-textMuted'
            }`}
          >
            {live.trailing}
          </span>
        </div>
        <div className="text-[10.5px] text-textMuted truncate">{live.line}</div>
        <div className="h-1 rounded-full bg-surface overflow-hidden mt-1">
          <div
            className={`h-full rounded-full transition-[width] duration-700 ${live.complete ? 'bg-success' : 'bg-accent'}`}
            style={{ width: `${live.percent}%` }}
          />
        </div>
      </div>
      {live.complete && (
        <button
          onClick={(e) => void claim(e)}
          disabled={claiming}
          className="shrink-0 px-2.5 py-1 rounded-full text-[11px] font-semibold bg-success/20 text-success disabled:opacity-60"
        >
          {claiming ? '…' : live.claim_label}
        </button>
      )}
    </div>
  );
};
