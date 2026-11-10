import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Reorder, useDragControls } from 'framer-motion';
import { ChartBar, ClockCounterClockwise, DotsSixVertical, Trophy, X } from 'phosphor-react';
import { managerAct, targetOf, type RecentPoll, type StreamManagerView } from '../../services/streamManager';
import { Tooltip } from '../ui/Tooltip';
import { BannerStatus, OverlayBanner, ShareRow } from './OverlayBanner';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { isImeComposing } from '../../utils/imeComposing';
import {
  codePoints,
  freshBuilder,
  growChoices,
  moveChoice,
  newChoiceId,
  runTool,
  setToolDraft,
  switchKind,
  toolKey,
  useTool,
  type BuilderChoice,
  type BuilderDraft,
  type BuilderKind,
} from './managerStore';
import { FIELD, FooterRow, FormShell, LABEL, MainButton, Segmented, useCloseSoon, useInstant, type Layout } from './managerKit';

/**
 * The builder for a Twitch poll or prediction: in the panel over the message
 * box, a page of the Manage channel window (where the side list picks Poll
 * or Prediction and the preview sits beside the fields) or the phone's
 * Manage channel sheet. The preview is the REAL OverlayBanner viewers get.
 * Twitch's limits come from Rust's view; Rust checks them again and runs the
 * create through its one door.
 */

interface Props {
  view: StreamManagerView;
  layout: Layout;
  /** The kind to open on (Start a prediction, /prediction). */
  initialKind?: BuilderKind;
  /** The window's page: the kind is the side list's, with no switch here. */
  fixedKind?: BuilderKind;
  onClose: () => void;
  /** Open in the Manage channel window (the panel only). */
  onExpand?: (kind: BuilderKind) => void;
}

const limitsOf = (view: StreamManagerView, kind: BuilderKind) => {
  const l = view.limits;
  return kind === 'poll'
    ? { title: l.poll_title, option: l.poll_choice, min: l.poll_choices_min, max: l.poll_choices_max, minSecs: l.poll_seconds_min, maxSecs: l.poll_seconds_max }
    : {
        title: l.prediction_title,
        option: l.prediction_outcome,
        min: l.prediction_outcomes_min,
        max: l.prediction_outcomes_max,
        minSecs: l.prediction_seconds_min,
        maxSecs: l.prediction_seconds_max,
      };
};

/** A count inside a box: quiet, amber in its last five, red past it. */
function BoxCount({ n, max, always }: { n: number; max: number; always: boolean }) {
  if (!always && n < max - 5) return null;
  const tone = n > max ? 'text-error' : n > max - 5 ? 'text-warning' : 'text-textSecondary';
  return <span className={`flex-shrink-0 text-[11px] font-semibold tabular-nums ${tone}`}>{`${n} / ${max}`}</span>;
}

export function PollPredictionComposer({ view, layout, initialKind, fixedKind, onClose, onExpand }: Props) {
  const ch = view.channel_id;
  const key = toolKey(ch, 'builder');
  const entry = useTool(key);
  const [startKind] = useState(initialKind);
  // The kind picked here with the switch, once there is one.
  const [picked, setPicked] = useState<BuilderKind | null>(null);
  const want = fixedKind ?? picked ?? startKind ?? null;
  const [blank] = useState(() => freshBuilder(want ?? 'poll'));
  const raw = (entry.draft as BuilderDraft | undefined) ?? blank;
  // The kind asked for (the menu's row, a slash command, the side list, the
  // switch) wins over a kept draft's. It is only shown here; the store gets
  // it with the next edit (`put` writes `d`), never from an effect, so two
  // builders open on one channel never write each other's kind back and forth.
  const d = want && raw.kind !== want ? switchKind(raw, want, limitsOf(view, want)) : raw;
  const limits = limitsOf(view, d.kind);
  const [showRecent, setShowRecent] = useState(false);
  const [tried, setTried] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);
  const closeSoon = useCloseSoon(onClose);
  const instant = useInstant();
  const sheet = layout === 'sheet';
  const big = layout === 'window';

  // On a phone the keyboard would cover the form the moment it opens.
  useEffect(() => {
    if (!sheet) titleRef.current?.focus();
  }, [sheet]);

  const put = (next: BuilderDraft) => {
    setTried(false);
    setToolDraft(key, next);
  };

  // Kept by Rust on this device (newest first); never written from here.
  const recents: RecentPoll[] = (d.kind === 'poll' ? view.recent_polls : view.recent_predictions) ?? [];

  const filled = d.choices.map((c) => c.text.trim()).filter(Boolean);
  const titleCount = codePoints(d.title.trim());
  const over = titleCount > limits.title || d.choices.some((c) => codePoints(c.text.trim()) > limits.option);
  const problem = !d.title.trim()
    ? 'Add a question'
    : filled.length < limits.min
      ? `Add at least ${limits.min} ${d.kind === 'poll' ? 'choices' : 'outcomes'}`
      : over
        ? 'Shorten what is over its count'
        : null;

  const durations = view.limits.durations.filter((x) => x.value >= limits.minSecs && x.value <= limits.maxSecs);

  const setChoice = (id: string, text: string) =>
    put({ ...d, choices: growChoices(d.choices.map((c) => (c.id === id ? { ...c, text } : c)), limits.max) });
  const removeChoice = (id: string) => put({ ...d, choices: d.choices.filter((c) => c.id !== id) });
  const focusChoice = (id: string) => requestAnimationFrame(() => document.getElementById(`sn-choice-${id}`)?.focus());

  const applyRecent = (r: RecentPoll) => {
    const choices: BuilderChoice[] = r.options.slice(0, limits.max).map((text) => ({ id: newChoiceId(), text }));
    put({ ...d, title: r.title, choices: growChoices(choices, limits.max) });
    setShowRecent(false);
    titleRef.current?.focus();
  };

  const start = async () => {
    if (problem) {
      setTried(true);
      return;
    }
    const title = d.title.trim();
    const sentKind = d.kind;
    const ok = await runTool(
      key,
      () =>
        managerAct(
          targetOf(view),
          sentKind === 'poll'
            ? { kind: 'create_poll', title, choices: filled, seconds: d.seconds, points_per_vote: d.points > 0 ? d.points : null }
            : { kind: 'create_prediction', title, outcomes: filled, seconds: d.seconds },
        ),
      { clearDraft: true },
    );
    // Rust keeps the recents: the answer's view already lists this one.
    if (!ok) return;
    if (!big) closeSoon();
  };

  const onEnter = (e: ReactKeyboardEvent, next?: () => void) => {
    if (e.key !== 'Enter' || isImeComposing(e)) return;
    e.preventDefault();
    if (!problem || !next) void start();
    else next();
  };

  const noun = d.kind === 'poll' ? 'poll' : 'prediction';
  const durationLabel = durations.find((x) => x.value === d.seconds)?.label ?? `${d.seconds}s`;

  const fields = (
    <div className="space-y-3.5">
      {!fixedKind && (
        <Segmented
          layout={layout}
          ariaLabel="Poll or prediction"
          value={d.kind}
          options={[
            { value: 'poll', label: 'Poll', icon: <ChartBar size={16} aria-hidden /> },
            { value: 'prediction', label: 'Prediction', icon: <Trophy size={16} aria-hidden /> },
          ]}
          onChange={(k) => {
            setPicked(k);
            put(switchKind(d, k, limitsOf(view, k)));
          }}
        />
      )}

      <div>
        <div className="mb-1.5 flex items-center justify-between">
          <label htmlFor={`sn-builder-title-${layout}`} className={LABEL}>
            {d.kind === 'poll' ? 'Question' : 'Prediction'}
          </label>
          {recents.length > 0 && (
            <Tooltip content="Reuse a recent one" side="top">
              <button
                type="button"
                onClick={() => setShowRecent((v) => !v)}
                aria-label="Recent"
                aria-expanded={showRecent}
                className="text-textSecondary transition-colors hover:text-textPrimary"
              >
                <ClockCounterClockwise size={13} />
              </button>
            </Tooltip>
          )}
        </div>
        <div className={`${FIELD} flex items-center gap-2 ${titleCount > limits.title ? '!border-error/60' : ''}`}>
          <input
            id={`sn-builder-title-${layout}`}
            ref={titleRef}
            value={d.title}
            onChange={(e) => put({ ...d, title: e.target.value })}
            onKeyDown={(e) => onEnter(e, () => d.choices[0] && focusChoice(d.choices[0].id))}
            placeholder={d.kind === 'poll' ? 'What should we play next?' : 'Do we win this one?'}
            className={`min-w-0 flex-1 bg-transparent text-textPrimary placeholder-textSecondary outline-none ${big ? 'h-11 text-[15px] font-semibold' : 'h-10 text-sm'}`}
          />
          <BoxCount n={titleCount} max={limits.title} always />
        </div>
        {showRecent && recents.length > 0 && (
          <div className="mt-1.5 overflow-hidden rounded-lg bg-surface/30 ring-1 ring-inset ring-textPrimary/[0.06]">
            {recents.map((r, i) => (
              <button
                key={`${r.title}-${i}`}
                type="button"
                onClick={() => applyRecent(r)}
                className="w-full px-2.5 py-1.5 text-left transition-colors hover:bg-textPrimary/[0.06]"
              >
                <span className="block truncate text-xs text-textPrimary">{r.title}</span>
                <span className="block truncate text-[10.5px] text-textSecondary">{r.options.join(' · ')}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      <div>
        <div className="mb-1.5 flex items-baseline justify-between">
          <span className={LABEL}>{d.kind === 'poll' ? 'Choices' : 'Outcomes'}</span>
          <span className="text-[11px] font-semibold tabular-nums text-textSecondary/80">
            {filled.length} / {limits.max}
          </span>
        </div>
        <Reorder.Group axis="y" values={d.choices} onReorder={(choices) => put({ ...d, choices })} className="space-y-1.5">
          {d.choices.map((c, i) => (
            <ChoiceRow
              key={c.id}
              choice={c}
              index={i}
              count={d.choices.length}
              max={limits.option}
              removable={d.choices.length > limits.min && !(i === d.choices.length - 1 && !c.text.trim())}
              instant={instant}
              onText={(t) => setChoice(c.id, t)}
              onRemove={() => removeChoice(c.id)}
              onMove={(delta) => {
                put({ ...d, choices: moveChoice(d.choices, i, delta) });
                focusChoice(c.id);
              }}
              onEnter={(e) => onEnter(e, () => (d.choices[i + 1] ? focusChoice(d.choices[i + 1].id) : undefined))}
            />
          ))}
        </Reorder.Group>
      </div>

      <div>
        <p className={`${LABEL} mb-1.5`}>Runs for</p>
        <Segmented
          layout={layout}
          ariaLabel="How long it runs"
          value={d.seconds}
          options={durations.map((x) => ({ value: x.value, label: x.label }))}
          onChange={(seconds) => put({ ...d, seconds })}
        />
      </div>

      {d.kind === 'poll' && (
        <div>
          <label htmlFor={`sn-builder-points-${layout}`} className={`${LABEL} mb-1.5 block`}>
            Extra votes with channel points
          </label>
          <div className={`${FIELD} flex items-center gap-2`}>
            <ChannelPointsIcon size={14} className="flex-shrink-0 text-accent" />
            <input
              id={`sn-builder-points-${layout}`}
              type="number"
              min={0}
              max={1000000}
              value={d.points || ''}
              onChange={(e) => put({ ...d, points: Math.max(0, Number(e.target.value) || 0) })}
              placeholder="Off"
              className="h-9 min-w-0 flex-1 bg-transparent text-sm text-textPrimary placeholder-textSecondary outline-none"
            />
            <span className="flex-shrink-0 text-[11.5px] text-textSecondary">per vote</span>
          </div>
        </div>
      )}
      {tried && problem && (
        <p role="status" className="text-[12px] text-warning">
          {problem}
        </p>
      )}
    </div>
  );

  const preview = (
    <div className="min-w-0">
      <p className={`${LABEL} mb-1.5`}>Preview</p>
      <div className="pointer-events-none opacity-95">
        <OverlayBanner
          icon={d.kind === 'poll' ? <ChartBar size={16} className="text-accent-on-surface" /> : <Trophy size={16} className="text-accent-on-surface" />}
          kind={d.kind === 'poll' ? 'Poll' : 'Prediction'}
          title={d.title.trim() || (d.kind === 'poll' ? 'Your question' : 'Your prediction')}
          isExpanded
          onToggleExpanded={() => {}}
          badges={<BannerStatus>{durationLabel}</BannerStatus>}
        >
          <div className="space-y-2 px-3 pb-3">
            {(filled.length ? filled : ['Option 1', 'Option 2']).map((o, i) => (
              <div key={i} className="block">
                <ShareRow title={o} percent={0} />
              </div>
            ))}
          </div>
        </OverlayBanner>
      </div>
    </div>
  );

  const footer = (
    <FooterRow layout={layout} hints={[['Enter', 'start'], ['Esc', 'close']]}>
      <MainButton
        layout={layout}
        phase={entry.phase}
        label={`Start ${noun}`}
        working="Starting"
        done="Started"
        onClick={() => void start()}
        disabled={!!problem}
      />
    </FooterRow>
  );

  return (
    <FormShell
      layout={layout}
      frame={{
        title: d.kind === 'poll' ? 'Start a poll' : 'Start a prediction',
        icon: d.kind === 'poll' ? <ChartBar size={16} /> : <Trophy size={16} />,
        onClose,
        onExpand: onExpand ? () => onExpand(d.kind) : undefined,
      }}
      footer={footer}
      problem={entry.phase === 'failed' ? entry.line : null}
    >
      {big ? (
        <div className="grid grid-cols-1 gap-6 min-[760px]:grid-cols-[minmax(0,1fr)_280px]">
          {fields}
          {preview}
        </div>
      ) : (
        <>
          {fields}
          {preview}
        </>
      )}
    </FormShell>
  );
}

/** One choice: a drag handle (the only place a drag starts, so a scroll in
 *  the sheet never moves a row), its box with its count, and Remove. */
function ChoiceRow({
  choice,
  index,
  count,
  max,
  removable,
  instant,
  onText,
  onRemove,
  onMove,
  onEnter,
}: {
  choice: BuilderChoice;
  index: number;
  count: number;
  max: number;
  removable: boolean;
  instant: boolean;
  onText: (t: string) => void;
  onRemove: () => void;
  onMove: (delta: number) => void;
  onEnter: (e: ReactKeyboardEvent) => void;
}) {
  const controls = useDragControls();
  const n = codePoints(choice.text.trim());
  return (
    <Reorder.Item
      value={choice}
      dragListener={false}
      dragControls={controls}
      transition={instant ? { duration: 0 } : undefined}
      // The grip and Remove sit inside the box, so every box in the column
      // shares the Question box's edges.
      className={`${FIELD} flex items-center gap-1 !pl-1 !pr-1.5 ${n > max ? '!border-error/60' : ''}`}
    >
      <span
        onPointerDown={(e) => controls.start(e)}
        aria-hidden
        className="grid h-8 w-6 flex-shrink-0 cursor-grab touch-none place-items-center text-textSecondary/70 hover:text-textSecondary active:cursor-grabbing"
      >
        <DotsSixVertical size={14} weight="bold" />
      </span>
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <input
          id={`sn-choice-${choice.id}`}
          value={choice.text}
          onChange={(e) => onText(e.target.value)}
          onKeyDown={(e) => {
            if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
              e.preventDefault();
              onMove(e.key === 'ArrowUp' ? -1 : 1);
              return;
            }
            onEnter(e);
          }}
          placeholder={`Option ${index + 1}`}
          aria-label={`Option ${index + 1} of ${count}`}
          className="h-9 min-w-0 flex-1 bg-transparent text-sm text-textPrimary placeholder-textSecondary outline-none"
        />
        <BoxCount n={n} max={max} always={false} />
      </div>
      <button
        type="button"
        onClick={onRemove}
        disabled={!removable}
        aria-label={`Remove option ${index + 1}`}
        className="grid h-7 w-6 flex-shrink-0 place-items-center rounded-full text-textSecondary transition-colors hover:text-error disabled:invisible"
      >
        <X size={13} />
      </button>
    </Reorder.Item>
  );
}

export default PollPredictionComposer;
