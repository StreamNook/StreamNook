import { useEffect, useMemo, useRef, useState } from 'react';
import { managerAct, type ActOutcome } from '../../services/streamManager';
import { BarChart3, Trophy, Plus, X, ChevronUp, ChevronDown, History } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Tooltip } from '../ui/Tooltip';
import { BannerStatus, OverlayBanner, ShareRow } from './OverlayBanner';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { Logger } from '../../utils/logger';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { IS_MOBILE } from '../../utils/platform';

/**
 * Builder for a Twitch poll or prediction, anchored above the chat input.
 *
 * Deliberately a popover on the shared chat glass rather than a modal: it opens
 * where your hands already are and leaves the stream and chat visible while you
 * fill it in. The preview underneath is the REAL OverlayBanner the viewers get,
 * not a mock-up, so what you see is what goes live.
 */

// Twitch's own limits. Enforced here so a create can't fail on something the
// form could have told you about first.
const LIMITS = {
  poll: { title: 60, option: 25, min: 2, max: 5, minSecs: 15, maxSecs: 1800 },
  prediction: { title: 45, option: 25, min: 2, max: 10, minSecs: 30, maxSecs: 1800 },
} as const;

const DURATIONS = [
  { label: '30s', value: 30 },
  { label: '1m', value: 60 },
  { label: '2m', value: 120 },
  { label: '5m', value: 300 },
  { label: '10m', value: 600 },
  { label: '30m', value: 1800 },
];

type Kind = 'poll' | 'prediction';

interface RecentEntry {
  title: string;
  options: string[];
}

interface Props {
  broadcasterId: string;
  channelLogin: string;
  /** Pre-selects a tab when opened from /poll or /prediction. */
  initialKind?: Kind;
  onClose: () => void;
  /** The stream manager's answer, so the Tools menu shows the running poll. */
  onOutcome?: (outcome: ActOutcome) => void;
  /** Drawn in a page (the phone's Manage channel sheet) rather than as a
   *  popover above the message box: no frame, no outside-press close. */
  inline?: boolean;
}

export function PollPredictionComposer({
  broadcasterId,
  channelLogin,
  initialKind = 'poll',
  onClose,
  onOutcome,
  inline = false,
}: Props) {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const addToast = useAppStore((s) => s.addToast);

  const [kind, setKind] = useState<Kind>(initialKind);
  const [title, setTitle] = useState('');
  const [options, setOptions] = useState<string[]>(['', '']);
  const [duration, setDuration] = useState(120);
  const [pointsPerVote, setPointsPerVote] = useState(0);
  const [busy, setBusy] = useState(false);
  const [showRecent, setShowRecent] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);

  const limits = LIMITS[kind];
  const recents: RecentEntry[] = useMemo(
    () => (kind === 'poll' ? settings.recent_polls : settings.recent_predictions) ?? [],
    [kind, settings.recent_polls, settings.recent_predictions],
  );

  useEffect(() => {
    // On a phone the keyboard would cover the form the moment it opens.
    if (!inline) titleRef.current?.focus();
  }, [inline]);

  // Escape closes; click-away closes. Matches the emote picker and mod menu.
  const markInside = useOutsidePress(!inline, [rootRef], onClose);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Switching tabs keeps what you've typed but trims to the new maximum, so
  // moving a 7-outcome prediction to a poll doesn't silently drop your work
  // without the count making it obvious.
  const switchKind = (next: Kind) => {
    setKind(next);
    setOptions((prev) => prev.slice(0, LIMITS[next].max));
    setDuration((d) => Math.min(Math.max(d, LIMITS[next].minSecs), LIMITS[next].maxSecs));
  };

  const setOption = (i: number, value: string) =>
    setOptions((prev) => prev.map((o, idx) => (idx === i ? value : o)));

  const moveOption = (i: number, delta: number) =>
    setOptions((prev) => {
      const next = [...prev];
      const j = i + delta;
      if (j < 0 || j >= next.length) return prev;
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });

  const filled = options.map((o) => o.trim()).filter(Boolean);
  const problem =
    !title.trim()
      ? 'Give it a title'
      : filled.length < limits.min
        ? `Needs at least ${limits.min} options`
        : null;

  const applyRecent = (entry: RecentEntry) => {
    setTitle(entry.title.slice(0, limits.title));
    setOptions(entry.options.slice(0, limits.max).map((o) => o.slice(0, limits.option)));
    setShowRecent(false);
    titleRef.current?.focus();
  };

  const rememberRecent = (entry: RecentEntry) => {
    const key = kind === 'poll' ? 'recent_polls' : 'recent_predictions';
    const prior = (recents ?? []).filter(
      (r) => r.title.toLowerCase() !== entry.title.toLowerCase(),
    );
    void updateSettings({ ...settings, [key]: [entry, ...prior].slice(0, 10) });
  };

  const submit = async () => {
    if (problem || busy) return;
    setBusy(true);
    try {
      // One Rust door: Helix for the broadcaster, Twitch's own GQL for a
      // moderator or editor, with Twitch's limits checked first.
      const outcome = await managerAct(
        broadcasterId,
        channelLogin,
        kind === 'poll'
          ? {
              kind: 'create_poll',
              title: title.trim(),
              choices: filled,
              seconds: duration,
              points_per_vote: pointsPerVote > 0 ? pointsPerVote : null,
            }
          : { kind: 'create_prediction', title: title.trim(), outcomes: filled, seconds: duration },
      );
      onOutcome?.(outcome);
      addToast(outcome.message, outcome.ok ? 'success' : 'error');
      if (!outcome.ok) return;
      rememberRecent({ title: title.trim(), options: filled });
      onClose();
    } catch (err) {
      Logger.error(`[Composer] create ${kind} failed:`, err);
      // The Rust layer passes Twitch's own message through, which names the
      // offending field on a 400.
      addToast(typeof err === 'string' ? err : `Could not start the ${kind}`, 'error');
    } finally {
      setBusy(false);
    }
  };

  const durationLabel = DURATIONS.find((d) => d.value === duration)?.label ?? `${duration}s`;

  return (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      className={
        inline
          ? 'flex flex-col'
          : 'glass-flyout rounded-xl !p-0 absolute bottom-full left-0 right-0 mb-2 z-50 flex flex-col max-h-[70vh] overflow-hidden'
      }
    >
      {/* Kind switch */}
      <div className={`flex items-center gap-1 ${inline ? 'pb-2' : 'p-2'}`}>
        {(['poll', 'prediction'] as const).map((k) => {
          const active = kind === k;
          const Icon = k === 'poll' ? BarChart3 : Trophy;
          return (
            <button
              key={k}
              onClick={() => switchKind(k)}
              className={`flex-1 flex h-7 items-center justify-center gap-1.5 ${
                active ? `${ACCENT_BUTTON} !text-xs` : `${GLAZE_PILL} text-xs`
              }`}
              style={active ? ACCENT_FILL : undefined}
            >
              <Icon size={14} />
              {k === 'poll' ? 'Poll' : 'Prediction'}
            </button>
          );
        })}
        {/* In a page the sheet's Back closes it. */}
        {!inline && (
          <Tooltip content="Close" side="top">
            <button
              onClick={onClose}
              className={`${GLAZE_PILL} grid h-7 w-7 flex-shrink-0 place-items-center`}
              aria-label="Close"
            >
              <X size={14} />
            </button>
          </Tooltip>
        )}
      </div>

      <div className={`overflow-y-auto scrollbar-thin pb-3 space-y-3 ${inline ? '' : 'px-3'}`}>
        {/* Title */}
        <div>
          <div className="flex items-baseline justify-between mb-1">
            <label className="text-[11px] font-semibold uppercase tracking-wider text-white/50">
              {kind === 'poll' ? 'Question' : 'Prediction'}
            </label>
            <div className="flex items-center gap-2">
              {recents.length > 0 && (
                <Tooltip content="Reuse a recent one" side="top">
                  <button
                    onClick={() => setShowRecent((v) => !v)}
                    className="text-textSecondary hover:text-textPrimary transition-colors"
                    aria-label="Recent"
                  >
                    <History size={13} />
                  </button>
                </Tooltip>
              )}
              <span className="text-[10px] tabular-nums text-textSecondary">
                {title.length}/{limits.title}
              </span>
            </div>
          </div>
          <input
            ref={titleRef}
            value={title}
            maxLength={limits.title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={kind === 'poll' ? 'What should we play next?' : 'Do we win this one?'}
            className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary"
          />
          {showRecent && recents.length > 0 && (
            <div className="mt-1.5 rounded-md overflow-hidden hairline-y">
              {recents.map((r, i) => (
                <button
                  key={`${r.title}-${i}`}
                  onClick={() => applyRecent(r)}
                  className="w-full text-left px-2.5 py-1.5 hover:bg-white/[0.06] transition-colors"
                >
                  <span className="block text-xs text-textPrimary truncate">{r.title}</span>
                  <span className="block text-[10px] text-textSecondary truncate">
                    {r.options.join(' · ')}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Options */}
        <div>
          <div className="flex items-baseline justify-between mb-1">
            <label className="text-[11px] font-semibold uppercase tracking-wider text-white/50">
              {kind === 'poll' ? 'Choices' : 'Outcomes'}
            </label>
            <span className="text-[10px] tabular-nums text-textSecondary">
              {filled.length}/{limits.max}
            </span>
          </div>
          <div className="space-y-1.5">
            {options.map((opt, i) => (
              <div key={i} className="flex items-center gap-1.5">
                <input
                  value={opt}
                  maxLength={limits.option}
                  onChange={(e) => setOption(i, e.target.value)}
                  placeholder={`Option ${i + 1}`}
                  className="glass-input flex-1 min-w-0 px-3 py-1.5 text-sm text-textPrimary placeholder-textSecondary"
                />
                <div className="flex flex-col">
                  <button
                    onClick={() => moveOption(i, -1)}
                    disabled={i === 0}
                    aria-label="Move up"
                    className="text-textSecondary hover:text-textPrimary disabled:opacity-25 transition-colors"
                  >
                    <ChevronUp size={12} />
                  </button>
                  <button
                    onClick={() => moveOption(i, 1)}
                    disabled={i === options.length - 1}
                    aria-label="Move down"
                    className="text-textSecondary hover:text-textPrimary disabled:opacity-25 transition-colors"
                  >
                    <ChevronDown size={12} />
                  </button>
                </div>
                <button
                  onClick={() => setOptions((p) => p.filter((_, idx) => idx !== i))}
                  disabled={options.length <= limits.min}
                  aria-label="Remove"
                  className="text-textSecondary hover:text-error disabled:opacity-25 transition-colors"
                >
                  <X size={13} />
                </button>
              </div>
            ))}
          </div>
          {options.length < limits.max && (
            <button
              onClick={() => setOptions((p) => [...p, ''])}
              className={`${GLAZE_PILL} mt-1.5 w-full flex items-center justify-center gap-1.5 ${IS_MOBILE ? 'min-h-[44px] text-[15px]' : 'h-7 text-xs'}`}
            >
              <Plus size={12} />
              Add {kind === 'poll' ? 'choice' : 'outcome'}
            </button>
          )}
        </div>

        {/* Duration */}
        <div>
          <label className="block text-[11px] font-semibold uppercase tracking-wider text-white/50 mb-1">
            Runs for {durationLabel}
          </label>
          <div className="flex gap-1">
            {DURATIONS.filter((d) => d.value >= limits.minSecs && d.value <= limits.maxSecs).map(
              (d) => (
                <button
                  key={d.value}
                  onClick={() => setDuration(d.value)}
                  className={`flex-1 h-7 ${
                    duration === d.value ? `${ACCENT_BUTTON} !text-xs` : `${GLAZE_PILL} text-xs`
                  }`}
                  style={duration === d.value ? ACCENT_FILL : undefined}
                >
                  {d.label}
                </button>
              ),
            )}
          </div>
        </div>

        {/* Poll-only: extra votes bought with points */}
        {kind === 'poll' && (
          <div>
            <label className="block text-[11px] font-semibold uppercase tracking-wider text-white/50 mb-1">
              Extra votes with channel points
            </label>
            <div className="flex items-center gap-2">
              <ChannelPointsIcon size={14} className="text-accent flex-shrink-0" />
              <input
                type="number"
                min={0}
                max={1000000}
                value={pointsPerVote || ''}
                onChange={(e) => setPointsPerVote(Math.max(0, Number(e.target.value) || 0))}
                placeholder="Off"
                className="glass-input flex-1 px-3 py-1.5 text-sm text-textPrimary placeholder-textSecondary"
              />
              <span className="text-[11px] text-textSecondary flex-shrink-0">per vote</span>
            </div>
          </div>
        )}

        {/* Live preview: the real banner, not a mock */}
        <div>
          <label className="block text-[11px] font-semibold uppercase tracking-wider text-white/50 mb-1">
            Preview
          </label>
          <div className="pointer-events-none opacity-95">
            <OverlayBanner
              icon={
                kind === 'poll' ? (
                  <BarChart3 className="w-4 h-4 text-accent-on-surface" />
                ) : (
                  <Trophy className="w-4 h-4 text-accent-on-surface" />
                )
              }
              kind={kind === 'poll' ? 'Poll' : 'Prediction'}
              title={title.trim() || (kind === 'poll' ? 'Your question' : 'Your prediction')}
              isExpanded
              onToggleExpanded={() => {}}
              badges={<BannerStatus>{durationLabel}</BannerStatus>}
            >
              <div className="px-3 pb-3 space-y-2">
                {(filled.length ? filled : ['Option 1', 'Option 2']).map((o, i) => (
                  <div key={i} className="block">
                    <ShareRow title={o} percent={0} />
                  </div>
                ))}
              </div>
            </OverlayBanner>
          </div>
        </div>
      </div>

      {/* Commit */}
      <div className={inline ? 'pt-0' : 'p-3 pt-0'}>
        <button
          onClick={submit}
          disabled={!!problem || busy}
          className={`${ACCENT_BUTTON} w-full disabled:cursor-not-allowed ${IS_MOBILE ? 'min-h-[44px] !text-[15px]' : 'h-9 !text-sm'}`}
          style={ACCENT_FILL}
        >
          {busy
            ? 'Starting…'
            : problem
              ? problem
              : `Start ${kind === 'poll' ? 'poll' : 'prediction'}`}
        </button>
      </div>
    </div>
  );
}

export default PollPredictionComposer;
