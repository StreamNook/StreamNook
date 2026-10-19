import { useState, useEffect, useMemo, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { Trophy, Users, CheckCircle2 } from 'lucide-react';
import { ChannelPointsIcon } from './ChannelPointsIcon';
import { BannerStatus, OverlayBanner, StackCardPresence, ShareRow } from './chat/OverlayBanner';
import { useStackPresence, type StackCardControl } from './chat/chatTopStack';
import { useAppStore } from '../stores/AppStore';
import { useChannelEmotes } from '../stores/chatConnectionStore';
import { buildEmoteNameMap, EmoteText } from '../utils/emoteText';
import { useDeadlineClock } from '../utils/deadlineClock';
import { Logger } from '../utils/logger';
import type { BetView, PredictionPush, PredictionView, PredictOutcome, ResultView } from '../types';

/**
 * The prediction card. Rust (services/predictions.rs) decides everything on
 * it: phase, percents, ratios, the viewer's stake, the result and every line
 * of copy. This view only draws the latest `PredictionView` and keeps the UI
 * state that is the viewer's alone: folded or open, the side being looked
 * at, the amount being typed, and whether the confirm is showing.
 */

interface PredictionOverlayProps {
  channelId?: string;
  channelLogin?: string;
  /** Its slot in the top-of-chat stack, when it sits in one. */
  stack?: StackCardControl;
}

/** How long to wait after the last side or amount change before asking Rust. */
const BET_ASK_DEBOUNCE_MS = 150;

// Each side keeps its color in its pick ring and its share bar. The words sit
// on the card itself, never on the color, so they read on every theme.
const SIDE_STYLES: Record<string, { ring: string; dot: string; bar: string }> = {
  blue: { ring: 'border-highlight-blue', dot: 'bg-highlight-blue', bar: 'bg-highlight-blue' },
  pink: { ring: 'border-highlight-pink', dot: 'bg-highlight-pink', bar: 'bg-highlight-pink' },
};
const OTHER_SIDE_STYLE = { ring: 'border-highlight-purple', dot: 'bg-highlight-purple', bar: 'bg-highlight-purple' };

const RESULT_STYLES: Record<ResultView['tone'], string> = {
  won: 'bg-success/20 border-success/50 text-success',
  lost: 'bg-error/20 border-error/50 text-error',
  refunded: 'bg-info/20 border-info/50 text-info',
  ended: 'bg-transparent border-accent-on-surface/60 text-accent-on-surface',
};

const PredictionOverlay = ({ channelId, channelLogin, stack }: PredictionOverlayProps) => {
  const { addToast, currentStream } = useAppStore();
  const currentChannelId = channelId || currentStream?.user_id;
  const currentChannelLogin = channelLogin || currentStream?.user_login;

  // Rust's card, tagged with the channel it belongs to so a channel switch
  // shows nothing until the new channel's card arrives.
  const [card, setCard] = useState<{ channelId: string; view: PredictionView | null } | null>(null);
  const view = card && card.channelId === currentChannelId ? card.view : null;
  const [expandedHere, setExpandedHere] = useState(true);
  const [selected, setSelected] = useState<string | null>(null);
  const [amountInput, setAmountInput] = useState('10');
  // Rust's answer about a bet, tagged with the prediction and side it was for,
  // so switching sides never shows the other side's button or confirm.
  const [betAnswer, setBetAnswer] = useState<{ key: string; bet: BetView | null } | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  // Outcome titles are plain text with no emote ranges, so match emote names
  // against the channel's set the same way pinned messages do.
  const channelEmotes = useChannelEmotes(currentChannelLogin, currentChannelId, 'twitch');
  const emoteMap = useMemo(() => buildEmoteNameMap(channelEmotes), [channelEmotes]);

  // Follow Rust's card for this channel: the getter covers a prediction
  // already running, the push every change after. A push that lands while the
  // getter is still out is newer, so the getter's answer must not replace it.
  useEffect(() => {
    if (!currentChannelId) return;
    let live = true;
    let pushed = false;
    const unlisten = listen<PredictionPush>('prediction-state', (event) => {
      if (event.payload.channel_id !== currentChannelId) return;
      pushed = true;
      setCard({ channelId: currentChannelId, view: event.payload.view });
    });
    invoke<PredictionPush>('get_channel_prediction', {
      channelId: currentChannelId,
      channelLogin: currentChannelLogin,
    })
      .then((push) => {
        if (live && !pushed && push.channel_id === currentChannelId) {
          setCard({ channelId: currentChannelId, view: push.view });
        }
      })
      .catch((err) => Logger.debug('[Prediction] First read failed:', err));
    return () => {
      live = false;
      // Tauri's unlisten throws if the registry entry is already gone (a
      // teardown after a webview reset); the listener is gone either way.
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [currentChannelId, currentChannelLogin]);

  // A new prediction opens the card and starts on a fresh pick. Updates to
  // the same one leave the viewer's fold and side alone. Adjusted during
  // render, not in an effect, so the new prediction's first frame is fresh.
  const predictionId = view?.prediction_id ?? null;
  const [seenPredictionId, setSeenPredictionId] = useState<string | null>(null);
  if (view && predictionId !== seenPredictionId) {
    setSeenPredictionId(predictionId);
    setExpandedHere(true);
    setSelected(view.picked ?? view.outcomes[0]?.id ?? null);
    setAmountInput('10');
    setConfirming(false);
  }

  // Once the viewer has staked, only that side is theirs to add to.
  const side = view?.picked ?? selected;
  const canPredict = view?.can_predict ?? false;
  const betKey = predictionId && side ? `${predictionId}:${side}` : null;
  const bet = canPredict && betAnswer && betAnswer.key === betKey ? betAnswer.bet : null;
  // The confirm only makes sense while predicting is still offered.
  const showConfirm = confirming && canPredict && bet !== null;

  // Ask Rust what this bet would say whenever the side, the amount or the card
  // changes (the pool moves the "win up to" line). Asking spends nothing.
  const askSeqRef = useRef(0);
  useEffect(() => {
    if (!currentChannelId || !view || !canPredict || !side || !betKey) return;
    const seq = ++askSeqRef.current;
    const timer = setTimeout(() => {
      invoke<BetView | null>('prediction_bet', {
        channelId: currentChannelId,
        outcomeId: side,
        amount: amountInput,
      })
        .then((answer) => {
          if (seq === askSeqRef.current) setBetAnswer({ key: betKey, bet: answer });
        })
        .catch((err) => Logger.debug('[Prediction] Bet read failed:', err));
    }, BET_ASK_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [currentChannelId, view, canPredict, side, betKey, amountInput]);

  const countdown = useDeadlineClock(view?.deadline_ms);

  // In a stack Rust decides which card is open, and this one reports itself so
  // it gets a slot; on its own it keeps its own fold.
  const released = useStackPresence(stack, predictionId);
  const isExpanded = stack ? stack.open : expandedHere;
  const toggle = stack ? stack.onToggle : () => setExpandedHere(!expandedHere);

  if (!view || (stack && !stack.placed)) return <StackCardPresence onLeft={released} />;

  const handlePredict = async () => {
    if (!currentChannelId || !side || !bet || submitting) return;
    setSubmitting(true);
    try {
      const outcome = await invoke<PredictOutcome>('predict', {
        channelId: currentChannelId,
        outcomeId: side,
        amount: bet.amount,
      });
      addToast(outcome.message, outcome.success ? 'success' : 'error');
      if (outcome.view && outcome.view.channel_id === currentChannelId) {
        setCard({ channelId: currentChannelId, view: outcome.view });
      }
    } catch (err) {
      Logger.error('[Prediction] predict failed:', err);
      addToast(String(err), 'error');
    } finally {
      // Back to the bet row either way: a refusal's reason shows there.
      setConfirming(false);
      setSubmitting(false);
    }
  };

  const pointsIcon = (className: string) =>
    view.points_icon ? (
      <img src={view.points_icon} alt={view.points_name} className="w-3 h-3" />
    ) : (
      <ChannelPointsIcon size={12} className={className} />
    );

  const status = countdown ? (
    <BannerStatus>{countdown}</BannerStatus>
  ) : view.badge ? (
    <BannerStatus mono={false}>{view.badge}</BannerStatus>
  ) : null;

  return (
    <StackCardPresence onLeft={released}>
    <OverlayBanner
      key="prediction"
      icon={<Trophy className="w-4 h-4 text-accent-on-surface" />}
      kind="Prediction"
      title={view.title}
      isExpanded={isExpanded}
      onToggleExpanded={toggle}
      badges={status}
      order={stack?.order}
    >
      {/* Outcomes */}
      <div className="px-1.5 pb-1.5 space-y-0.5">
        {view.outcomes.map((outcome) => {
          const style = SIDE_STYLES[outcome.color] ?? OTHER_SIDE_STYLE;
          const isChosen = canPredict && side === outcome.id;
          const closedToViewer = !canPredict || (view.picked !== null && view.picked !== outcome.id);
          return (
            <button
              key={outcome.id}
              onClick={() => {
                if (closedToViewer) return;
                setSelected(outcome.id);
                setConfirming(false);
              }}
              disabled={closedToViewer}
              aria-pressed={isChosen}
              className={`w-full block text-left px-1.5 py-1.5 rounded-md transition-colors ${
                isChosen ? 'bg-textPrimary/[0.06]' : ''
              } ${closedToViewer ? 'cursor-default' : 'cursor-pointer hover:bg-textPrimary/[0.05]'} ${
                closedToViewer && !outcome.winner && !outcome.mine ? 'opacity-70' : ''
              }`}
            >
              <ShareRow
                title={<EmoteText text={outcome.title} emoteMap={emoteMap} keyPrefix={`pred-${outcome.id}`} />}
                strong={isChosen || outcome.winner || outcome.mine}
                percent={outcome.percent}
                barClass={style.bar}
                mark={
                  outcome.winner ? (
                    <Trophy className="w-3.5 h-3.5 text-success" />
                  ) : outcome.mine ? (
                    <CheckCircle2 className="w-3.5 h-3.5 text-textPrimary" />
                  ) : canPredict ? (
                    <span className={`w-3.5 h-3.5 rounded-full border-2 ${style.ring} flex items-center justify-center`}>
                      {isChosen && <span className={`w-1.5 h-1.5 rounded-full ${style.dot}`} />}
                    </span>
                  ) : undefined
                }
                trailing={
                  <>
                    <span className="font-mono text-textSecondary">{outcome.ratio}</span>
                    <span className="flex items-center gap-1 text-textMuted">
                      <Users className="w-3 h-3" />
                      {outcome.people_label}
                    </span>
                    <span className="flex items-center gap-1 text-textMuted">
                      {pointsIcon('text-textMuted')}
                      {outcome.points_label}
                    </span>
                    <span className="w-9 text-right font-semibold text-textPrimary">{outcome.percent}%</span>
                  </>
                }
              />
            </button>
          );
        })}
      </div>

      {/* The bet, then a confirm before any points are spent. */}
      {canPredict && side && (
        <div className="px-3 pb-3">
          {showConfirm && bet ? (
            <div className="p-3 bg-backgroundSecondary rounded-lg border border-border text-center">
              <p className="text-sm font-semibold text-textPrimary">{bet.confirm_title}</p>
              <p className="mt-1 text-xs text-textSecondary">{bet.confirm_line}</p>
              {bet.confirm_note && <p className="mt-1 text-xs text-textSecondary/80">{bet.confirm_note}</p>}
              <div className="mt-3 flex gap-2">
                <button
                  onClick={() => setConfirming(false)}
                  disabled={submitting}
                  className="flex-1 px-3 py-1.5 rounded text-xs font-semibold bg-background border border-border text-textPrimary hover:bg-backgroundSecondary transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handlePredict}
                  disabled={submitting || !bet.ok}
                  className={`flex-1 px-3 py-1.5 rounded text-xs font-bold transition-colors ${
                    submitting || !bet.ok
                      ? 'bg-background border border-border text-textSecondary cursor-not-allowed'
                      : 'bg-accent-on-surface hover:opacity-90 text-background'
                  }`}
                >
                  {submitting ? '...' : 'Predict'}
                </button>
              </div>
            </div>
          ) : (
            <>
              <div className="flex items-center gap-2 p-2 bg-backgroundSecondary rounded-lg border border-border">
                <input
                  type="text"
                  inputMode="numeric"
                  value={amountInput}
                  onChange={(e) => setAmountInput(e.target.value)}
                  // Show the amount Rust will actually use once typing stops.
                  onBlur={() => {
                    if (bet) setAmountInput(String(bet.amount));
                  }}
                  className="w-20 px-2 py-1.5 glass-input text-textPrimary text-sm font-medium focus:outline-none"
                  placeholder="Amount"
                />
                <div className="flex gap-1">
                  {(bet?.presets ?? []).map((preset) => (
                    <button
                      key={preset.label}
                      onClick={() => setAmountInput(String(preset.amount))}
                      className={`px-2 py-1.5 text-xs font-medium rounded transition-colors border ${
                        bet?.amount === preset.amount
                          ? 'bg-transparent border-accent-on-surface text-accent-on-surface'
                          : 'bg-background border-border text-textSecondary hover:bg-backgroundSecondary'
                      }`}
                    >
                      {preset.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="mt-2">
                {bet?.problem ? (
                  <p className="py-1.5 text-center text-xs font-medium text-textSecondary">{bet.problem}</p>
                ) : (
                  <button
                    onClick={() => setConfirming(true)}
                    disabled={!bet || !bet.ok}
                    className={`w-full px-3 py-1.5 rounded text-xs font-bold transition-colors ${
                      bet && bet.ok
                        ? 'bg-accent-on-surface hover:opacity-90 text-background'
                        : 'bg-background border border-border text-textSecondary cursor-not-allowed'
                    }`}
                  >
                    {bet?.button ?? '...'}
                  </button>
                )}
              </div>
              {bet && (
                <div className="mt-1.5 flex items-center justify-between text-xs text-textSecondary">
                  <span>{bet.win_line}</span>
                  {bet.balance_line && (
                    <span className="flex items-center gap-1">
                      {pointsIcon('text-textSecondary')}
                      {bet.balance_line}
                    </span>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* What the viewer staked, and how it stands */}
      {view.status_line && (
        <div className="px-3 pb-3">
          <div className="py-2 px-3 bg-backgroundSecondary border border-border rounded-lg flex items-center justify-center gap-2">
            {pointsIcon('text-textSecondary')}
            <span className="text-textPrimary text-sm font-medium">{view.status_line}</span>
          </div>
        </div>
      )}

      {view.result && (
        <div className="px-3 pb-3">
          <div className={`py-2.5 px-3 border rounded-lg text-center text-sm font-semibold ${RESULT_STYLES[view.result.tone] ?? RESULT_STYLES.ended}`}>
            {view.result.line}
          </div>
        </div>
      )}

      <p className="px-3 pb-3 flex items-center justify-center gap-1 text-xs text-textSecondary">
        <Users className="w-3 h-3" />
        {view.totals_line}
      </p>
    </OverlayBanner>
    </StackCardPresence>
  );
};

export default PredictionOverlay;
