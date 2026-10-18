import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { BarChart3, Users, CheckCircle2, Trophy } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { OverlayBanner } from './chat/OverlayBanner';
import { useDeadlineClock } from '../utils/deadlineClock';
import { Logger } from '../utils/logger';
import type { PollPush, PollView } from '../types';

/**
 * The poll card. Rust (services/poll_view.rs) decides everything on it:
 * phase, percents, the winner, the viewer's vote and every line of copy. This
 * view draws the latest `PollView` and keeps only the viewer's own UI state:
 * folded or open, and a vote it is still waiting on Rust to confirm.
 */

interface PollOverlayProps {
  channelId?: string;
  channelLogin?: string;
}

const PollOverlay = ({ channelId }: PollOverlayProps) => {
  const { addToast, currentStream } = useAppStore();
  const currentChannelId = channelId || currentStream?.user_id;

  // Mirrors the pinned banner's `pinned_start_collapsed`: some people want polls
  // out of the way by default rather than eating the top of chat.
  const pollsStartCollapsed = useAppStore(
    (s) => s.settings.chat_design?.polls_start_collapsed ?? false,
  );

  // Rust's card, tagged with the channel it belongs to so a channel switch
  // shows nothing until the new channel's card arrives.
  const [card, setCard] = useState<{ channelId: string; view: PollView | null } | null>(null);
  const view = card && card.channelId === currentChannelId ? card.view : null;
  const [isExpanded, setIsExpanded] = useState(!pollsStartCollapsed);
  // A tap marks its choice at once; Rust's next push confirms it, and a
  // refusal takes the mark back.
  const [pending, setPending] = useState<{ pollId: string; choiceId: string } | null>(null);

  // Follow Rust's card for this channel: the getter covers a poll already
  // running (and starts Rust following it), the push every change after. A
  // push that lands while the getter is out is newer, so it wins.
  useEffect(() => {
    if (!currentChannelId) return;
    let live = true;
    let pushed = false;
    const unlisten = listen<PollPush>('poll-state', (event) => {
      if (event.payload.channel_id !== currentChannelId) return;
      pushed = true;
      setCard({ channelId: currentChannelId, view: event.payload.view });
    });
    invoke<PollView | null>('watch_channel_poll_view', { channelId: currentChannelId })
      .then((poll) => {
        if (live && !pushed && (!poll || poll.channel_id === currentChannelId)) {
          setCard({ channelId: currentChannelId, view: poll });
        }
      })
      .catch((err) => Logger.debug('[Poll] First read failed:', err));
    return () => {
      live = false;
      // Tauri's unlisten throws if the registry entry is already gone (a
      // teardown after a webview reset); the listener is gone either way.
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [currentChannelId]);

  // A brand new poll opens, unless polls are set to start collapsed. Updates
  // to the same poll never unfold one the viewer folded. Adjusted during
  // render so the new poll's first frame already has its fold.
  const pollId = view?.poll_id ?? null;
  const [seenPollId, setSeenPollId] = useState<string | null>(null);
  if (pollId && pollId !== seenPollId) {
    setSeenPollId(pollId);
    setIsExpanded(!pollsStartCollapsed);
  }

  const countdown = useDeadlineClock(view?.deadline_ms);

  if (!view) return null;

  const pendingChoice = pending?.pollId === view.poll_id ? pending.choiceId : null;
  const canTap = view.can_vote && !pendingChoice;

  const handleVote = async (choiceId: string) => {
    if (!currentChannelId || !canTap) return;
    const vote = { pollId: view.poll_id, choiceId };
    setPending(vote);
    try {
      await invoke('vote_on_poll', { pollId: vote.pollId, choiceId, channelId: currentChannelId });
    } catch (err) {
      Logger.error('[Poll] Vote failed:', err);
      addToast(String(err), 'error');
      setPending((p) => (p?.pollId === vote.pollId && p.choiceId === choiceId ? null : p));
    }
  };

  const timeBadge = countdown ? (
    <span className="text-xs font-mono font-bold text-warning bg-warning/20 border border-warning/40 px-1.5 py-1 rounded-md">
      {countdown}
    </span>
  ) : view.badge ? (
    <span className="text-xs font-medium text-accent bg-accent/20 border border-accent/40 px-1.5 py-1 rounded-md">
      {view.badge}
    </span>
  ) : null;

  return (
    <OverlayBanner
      icon={<BarChart3 className="w-4 h-4 text-accent" />}
      title={view.title}
      isExpanded={isExpanded}
      onToggleExpanded={() => setIsExpanded(!isExpanded)}
      badges={timeBadge}
    >
      <div className="p-3 space-y-2">
        {view.choices.map((choice) => {
          const isMine = choice.mine || pendingChoice === choice.id;
          return (
            <button
              key={choice.id}
              onClick={() => handleVote(choice.id)}
              disabled={!canTap}
              className={`w-full relative p-2.5 rounded-lg border transition-all overflow-hidden ${
                choice.winner
                  ? 'bg-accent/20 border-accent'
                  : isMine
                    ? 'bg-accent/20 border-accent/70'
                    : 'bg-backgroundSecondary border-border ' +
                      (canTap ? 'hover:bg-accent/10 hover:border-accent/40 cursor-pointer' : 'cursor-default')
              }`}
            >
              {/* Vote-share bar */}
              <div
                className={`absolute inset-y-0 left-0 rounded-md opacity-25 transition-[width] duration-500 ${
                  choice.winner ? 'bg-accent' : 'bg-accent/70'
                }`}
                style={{ width: `${choice.percent}%` }}
              />
              <div className="relative flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 min-w-0">
                  {choice.winner && <Trophy className="w-3.5 h-3.5 text-accent flex-shrink-0" />}
                  {isMine && !choice.winner && <CheckCircle2 className="w-3.5 h-3.5 text-accent flex-shrink-0" />}
                  <span className="font-semibold text-textPrimary text-sm truncate">{choice.title}</span>
                </div>
                <div className="flex items-center gap-3 text-xs text-textSecondary flex-shrink-0">
                  <span className="flex items-center gap-1">
                    <Users className="w-3 h-3" />
                    {choice.votes_label}
                  </span>
                  <span className="font-bold text-sm text-textPrimary">{choice.percent}%</span>
                </div>
              </div>
            </button>
          );
        })}
      </div>

      <div className="px-3 pb-3">
        {view.result_line && (
          <div className="mb-2 py-2 px-3 bg-accent/15 border border-accent/40 rounded-lg flex items-center justify-center gap-2">
            <Trophy className="w-4 h-4 text-accent" />
            <span className="text-accent text-sm font-semibold">{view.result_line}</span>
          </div>
        )}
        <p className="text-center text-xs text-textSecondary">{view.footer}</p>
        <div className="mt-2 flex items-center justify-center gap-1 text-xs text-textSecondary">
          <Users className="w-3 h-3" />
          {view.voters_line}
        </div>
      </div>
    </OverlayBanner>
  );
};

export default PollOverlay;
