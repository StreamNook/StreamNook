import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { BarChart3, Users, CheckCircle2, Trophy } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { BannerStatus, OverlayBanner, StackCardPresence, ShareRow } from './chat/OverlayBanner';
import { useStackPresence, type StackCardControl } from './chat/chatTopStack';
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
  /** Its slot in the top-of-chat stack, when it sits in one. */
  stack?: StackCardControl;
}

const PollOverlay = ({ channelId, stack }: PollOverlayProps) => {
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
  const [expandedHere, setExpandedHere] = useState(!pollsStartCollapsed);
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
    setExpandedHere(!pollsStartCollapsed);
  }

  const countdown = useDeadlineClock(view?.deadline_ms);

  // In a stack Rust decides which card is open, and this one reports itself so
  // it gets a slot; on its own it keeps its own fold.
  const released = useStackPresence(stack, pollId);
  const isExpanded = stack ? stack.open : expandedHere;
  const toggle = stack ? stack.onToggle : () => setExpandedHere(!expandedHere);

  if (!view || (stack && !stack.placed)) return <StackCardPresence onLeft={released} />;

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

  const status = countdown ? (
    <BannerStatus>{countdown}</BannerStatus>
  ) : view.badge ? (
    <BannerStatus mono={false}>{view.badge}</BannerStatus>
  ) : null;

  return (
    <StackCardPresence onLeft={released}>
    <OverlayBanner
      key="poll"
      icon={<BarChart3 className="w-4 h-4 text-accent-on-surface" />}
      kind="Poll"
      title={view.title}
      isExpanded={isExpanded}
      onToggleExpanded={toggle}
      badges={status}
      order={stack?.order}
    >
      <div className="px-1.5 space-y-0.5">
        {view.choices.map((choice) => {
          const isMine = choice.mine || pendingChoice === choice.id;
          return (
            <button
              key={choice.id}
              onClick={() => handleVote(choice.id)}
              disabled={!canTap}
              aria-pressed={isMine}
              className={`w-full block text-left px-1.5 py-1.5 rounded-md transition-colors ${
                canTap ? 'hover:bg-textPrimary/[0.05] active:bg-textPrimary/[0.08] cursor-pointer' : 'cursor-default'
              }`}
            >
              <ShareRow
                title={choice.title}
                strong={choice.winner || isMine}
                percent={choice.percent}
                mark={
                  choice.winner ? (
                    <Trophy className="w-3.5 h-3.5 text-accent-on-surface" />
                  ) : isMine ? (
                    <CheckCircle2 className="w-3.5 h-3.5 text-accent-on-surface" />
                  ) : undefined
                }
                trailing={
                  <>
                    <span className="flex items-center gap-1 text-textMuted">
                      <Users className="w-3 h-3" />
                      {choice.votes_label}
                    </span>
                    <span className="w-9 text-right font-semibold text-textPrimary">{choice.percent}%</span>
                  </>
                }
              />
            </button>
          );
        })}
      </div>

      <div className="px-3 pt-1.5 pb-3">
        {view.result_line && (
          <p className="mb-1 flex items-center gap-1.5 text-[12px] font-semibold text-accent-on-surface">
            <Trophy className="w-3.5 h-3.5 shrink-0" />
            <span className="min-w-0 truncate">{view.result_line}</span>
          </p>
        )}
        <p className="flex items-center gap-2 text-[11px] text-textMuted">
          <span className="flex-1 min-w-0 truncate">{view.footer}</span>
          <span className="shrink-0 flex items-center gap-1 tabular-nums">
            <Users className="w-3 h-3" />
            {view.voters_line}
          </span>
        </p>
      </div>
    </OverlayBanner>
    </StackCardPresence>
  );
};

export default PollOverlay;
