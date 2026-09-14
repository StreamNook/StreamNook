import { useState, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { BarChart3, Users, CheckCircle2, Trophy } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { OverlayBanner } from './chat/OverlayBanner';
import { Logger } from '../utils/logger';

interface PollChoice {
  id: string;
  title: string;
  total_votes: number;
  total_voters: number;
}

interface PollData {
  channel_id: string;
  poll_id: string;
  title: string;
  status: string; // ACTIVE | COMPLETED
  duration_seconds: number;
  remaining_ms: number;
  total_voters: number;
  total_votes: number;
  started_at: string;
  channel_points_voting: boolean;
  channel_points_cost: number;
  choices: PollChoice[];
}

interface PollOverlayProps {
  channelId?: string;
  channelLogin?: string;
}

const PollOverlay = ({ channelId }: PollOverlayProps) => {
  const [activePoll, setActivePoll] = useState<PollData | null>(null);
  const [votedChoiceId, setVotedChoiceId] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [timeRemaining, setTimeRemaining] = useState<number>(0); // seconds
  // Mirrors the pinned banner's `pinned_start_collapsed`: some people want polls
  // out of the way by default rather than eating the top of chat.
  const pollsStartCollapsed = useAppStore(
    (s) => s.settings.chat_design?.polls_start_collapsed ?? false,
  );
  const [isExpanded, setIsExpanded] = useState(!pollsStartCollapsed);

  // Ref mirrors votedChoiceId so the completion listener sees the latest value
  // without re-subscribing (same pattern as PredictionOverlay).
  const votedChoiceIdRef = useRef(votedChoiceId);
  useEffect(() => { votedChoiceIdRef.current = votedChoiceId; }, [votedChoiceId]);

  const { addToast, currentStream } = useAppStore();

  const currentChannelId = channelId || currentStream?.user_id;

  const isClosed = activePoll?.status === 'COMPLETED';
  const isLocked = isClosed || timeRemaining <= 0 || votedChoiceId !== null;

  // Reset when the watched channel changes.
  useEffect(() => {
    setActivePoll(null);
    setVotedChoiceId(null);
    setTimeRemaining(0);
  }, [currentChannelId]);

  // Listen for poll lifecycle events from the PubSub backend.
  useEffect(() => {
    const applyPoll = (poll: PollData) => {
      setActivePoll(poll);
      setTimeRemaining(Math.max(0, Math.round((poll.remaining_ms || 0) / 1000)));
      // A brand new poll opens, unless you asked for polls to start collapsed.
      setIsExpanded(!pollsStartCollapsed);
    };

    const unlistenCreated = listen<PollData>('poll-created', (event) => {
      const poll = event.payload;
      if (currentChannelId && poll.channel_id === currentChannelId) {
        Logger.debug('[Poll] Created:', poll.title);
        setVotedChoiceId(null);
        applyPoll(poll);
      }
    });

    const unlistenUpdated = listen<PollData>('poll-updated', (event) => {
      const poll = event.payload;
      if (!currentChannelId || poll.channel_id !== currentChannelId) return;

      // Late-join: first time we hear about an in-progress poll is an update.
      setActivePoll(prev => {
        if (!prev || prev.poll_id !== poll.poll_id) {
          Logger.debug('[Poll] Late-join via update:', poll.title);
          return poll;
        }
        // Merge live counts while keeping our local state.
        return { ...prev, ...poll };
      });
      setTimeRemaining(Math.max(0, Math.round((poll.remaining_ms || 0) / 1000)));
      // Deliberately NOT re-expanding here. This listener fires on every vote,
      // so forcing it open meant a poll you collapsed sprang back open the
      // moment anyone else voted. Only a NEW poll opens itself (see applyPoll).
    });

    const unlistenCompleted = listen<PollData>('poll-completed', (event) => {
      const poll = event.payload;
      if (!currentChannelId || poll.channel_id !== currentChannelId) return;

      Logger.debug('[Poll] Completed:', poll.title);
      setActivePoll(prev => (prev ? { ...prev, ...poll, status: 'COMPLETED' } : poll));
      setTimeRemaining(0);

      // Announce the winner, then clear the overlay after a beat.
      const winner = [...poll.choices].sort((a, b) => b.total_votes - a.total_votes)[0];
      if (winner && poll.total_votes > 0) {
        const userWon = votedChoiceIdRef.current === winner.id;
        addToast(
          userWon
            ? `Poll ended. Your pick "${winner.title}" won!`
            : `Poll ended. Winner: "${winner.title}"`,
          'success'
        );
      }

      setTimeout(() => {
        setActivePoll(null);
        setVotedChoiceId(null);
      }, 5000);
    });

    return () => {
      // A teardown unlisten can throw if the registry entry is already gone;
      // swallow it so it doesn't surface as an uncaught rejection.
      unlistenCreated.then(fn => fn()).catch(() => {});
      unlistenUpdated.then(fn => fn()).catch(() => {});
      unlistenCompleted.then(fn => fn()).catch(() => {});
    };
  }, [currentChannelId, addToast, pollsStartCollapsed]);

  // Countdown timer.
  useEffect(() => {
    if (!activePoll || isClosed || timeRemaining <= 0) return;
    const timer = setInterval(() => {
      setTimeRemaining(prev => (prev <= 1 ? 0 : prev - 1));
    }, 1000);
    return () => clearInterval(timer);
  }, [activePoll, isClosed, timeRemaining]);

  const handleVote = async (choiceId: string) => {
    if (!activePoll || isSubmitting || isLocked) return;

    setIsSubmitting(true);
    // Optimistic: lock in the choice immediately; live counts arrive via PubSub.
    setVotedChoiceId(choiceId);

    try {
      await invoke('vote_on_poll', {
        pollId: activePoll.poll_id,
        choiceId,
        channelId: currentChannelId,
      });
      const title = activePoll.choices.find(c => c.id === choiceId)?.title;
      addToast(`Vote cast for "${title}"`, 'success');
    } catch (err: any) {
      Logger.error('[Poll] Vote failed:', err);
      addToast(`Failed to vote: ${err}`, 'error');
      setVotedChoiceId(null); // roll back so the user can retry
    } finally {
      setIsSubmitting(false);
    }
  };

  const formatTime = (seconds: number) => {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  };

  const totalVotes = activePoll?.choices.reduce((sum, c) => sum + c.total_votes, 0) || 0;
  const getPercentage = (choice: PollChoice) =>
    totalVotes === 0 ? 0 : Math.round((choice.total_votes / totalVotes) * 100);

  // Winner (only meaningful once closed) for result highlighting.
  const winningChoiceId = isClosed && totalVotes > 0
    ? [...(activePoll?.choices || [])].sort((a, b) => b.total_votes - a.total_votes)[0]?.id
    : null;

  if (!activePoll) return null;

  return (
    <OverlayBanner
      icon={<BarChart3 className="w-4 h-4 text-accent" />}
      title={activePoll.title}
      isExpanded={isExpanded}
      onToggleExpanded={() => setIsExpanded(!isExpanded)}
      badges={
        isClosed ? (
          <span className="text-xs font-medium text-accent bg-accent/20 border border-accent/40 px-1.5 py-1 rounded-md">
            Final
          </span>
        ) : timeRemaining > 0 ? (
          <span className="text-xs font-mono font-bold text-warning bg-warning/20 border border-warning/40 px-1.5 py-1 rounded-md">
            {formatTime(timeRemaining)}
          </span>
        ) : (
          <span className="text-xs font-medium text-error bg-error/20 border border-error/40 px-1.5 py-1 rounded-md">
            Closed
          </span>
        )
      }
    >
      <div className="p-3 space-y-2">
        {activePoll.choices.map((choice) => {
          const percentage = getPercentage(choice);
          const isVoted = votedChoiceId === choice.id;
          const isWinner = winningChoiceId === choice.id;
          const clickable = !isLocked;

          return (
            <button
              key={choice.id}
              onClick={() => clickable && handleVote(choice.id)}
              disabled={!clickable}
              className={`w-full relative p-2.5 rounded-lg border transition-all overflow-hidden ${
                isWinner
                  ? 'bg-accent/20 border-accent'
                  : isVoted
                    ? 'bg-accent/20 border-accent/70'
                    : 'bg-backgroundSecondary border-border ' + (clickable ? 'hover:bg-accent/10 hover:border-accent/40 cursor-pointer' : 'cursor-default')
              }`}
            >
              {/* Vote-share bar */}
              <div
                className={`absolute inset-y-0 left-0 rounded-md opacity-25 transition-[width] duration-500 ${isWinner ? 'bg-accent' : 'bg-accent/70'}`}
                style={{ width: `${percentage}%` }}
              />
              <div className="relative flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 min-w-0">
                  {isWinner && <Trophy className="w-3.5 h-3.5 text-accent flex-shrink-0" />}
                  {isVoted && !isWinner && <CheckCircle2 className="w-3.5 h-3.5 text-accent flex-shrink-0" />}
                  <span className="font-semibold text-textPrimary text-sm truncate">{choice.title}</span>
                </div>
                <div className="flex items-center gap-3 text-xs text-textSecondary flex-shrink-0">
                  <span className="flex items-center gap-1">
                    <Users className="w-3 h-3" />
                    {choice.total_votes.toLocaleString()}
                  </span>
                  <span className="font-bold text-sm text-textPrimary">{percentage}%</span>
                </div>
              </div>
            </button>
          );
        })}
      </div>

      {/* Footer status line */}
      <div className="px-3 pb-3">
        {votedChoiceId && !isClosed && (
          <div className="py-2 px-3 bg-accent/15 border border-accent/40 rounded-lg flex items-center justify-center gap-2">
            <CheckCircle2 className="w-4 h-4 text-accent" />
            <span className="text-accent text-sm font-semibold">Vote counted</span>
          </div>
        )}
        {!votedChoiceId && !isLocked && (
          <p className="text-center text-xs text-textSecondary">
            Tap a choice to vote
            {activePoll.channel_points_voting ? ' (free votes only)' : ''}
          </p>
        )}
        <div className="mt-2 flex items-center justify-center gap-1 text-xs text-textSecondary">
          <Users className="w-3 h-3" />
          {activePoll.total_voters.toLocaleString()} {activePoll.total_voters === 1 ? 'voter' : 'voters'}
        </div>
      </div>
    </OverlayBanner>
  );
};

export default PollOverlay;
