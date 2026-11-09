import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import {
  Check,
  ChevronDown,
  CircleAlert,
  LoaderCircle,
  Maximize2,
  Megaphone,
  Minimize2,
  Pencil,
  Search,
  Send,
  SmilePlus,
  Tv,
  Undo2,
  X,
} from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { usePerformanceQuiet } from '../../utils/lowPower';
import { TitleEmojiPicker } from './TitleEmojiPicker';
import { Tooltip } from '../ui/Tooltip';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { isImeComposing } from '../../utils/imeComposing';
import { IS_MOBILE } from '../../utils/platform';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import {
  checkStreamInfo,
  countdown,
  getRewardQueue,
  managerAct,
  searchRaidTargets,
  type ActOutcome,
  type Category,
  type InfoCheck,
  type ManagerAction,
  type QueueView,
  type RaidTarget,
  type StreamInfoDraft,
  type StreamInfoView,
  type StreamManagerView,
} from '../../services/streamManager';

/**
 * The stream manager's builders, anchored above the message box like the
 * poll builder: stream info, raid, ad break, rewards queue and announcements.
 * Everything here draws Rust's view (services/stream_manager.rs) and every
 * write goes through its one door; this file only lays it out.
 */

export type ManagerPanelKind = 'info' | 'raid' | 'ads' | 'rewards' | 'announce';

interface Props {
  kind: ManagerPanelKind;
  view: StreamManagerView;
  queue: QueueView | null;
  onQueue: (q: QueueView) => void;
  onOutcome: (o: ActOutcome) => void;
  onClose: () => void;
}

const LABEL = 'text-[11px] font-semibold uppercase tracking-wider text-textSecondary';
/** A full-size action's height and text: finger-sized in the phone's sheet. */
const BIG = IS_MOBILE ? 'min-h-[44px] !text-[15px]' : 'h-9 !text-sm';

export const TITLES: Record<ManagerPanelKind, { title: string; icon: ReactNode }> = {
  info: { title: 'Stream info', icon: <Pencil size={14} /> },
  raid: { title: 'Raid a channel', icon: <Send size={14} /> },
  ads: { title: 'Ad break', icon: <Tv size={14} /> },
  rewards: { title: 'Rewards queue', icon: <ChannelPointsIcon size={14} /> },
  announce: { title: 'Announce', icon: <Megaphone size={14} /> },
};

/**
 * One runner for every write: Rust answers with the line to show and the view
 * after it. `onDone` runs after a write that succeeded and asked to close.
 */
export function useManagerAct(view: StreamManagerView, onOutcome: (o: ActOutcome) => void, onDone: () => void) {
  const addToast = useAppStore((s) => s.addToast);
  const [busy, setBusy] = useState(false);
  const act: Act = async (action, closeOnSuccess = false) => {
    if (busy) return false;
    setBusy(true);
    try {
      const outcome = await managerAct(view.channel_id, view.channel_login, action);
      onOutcome(outcome);
      addToast(outcome.message, outcome.ok ? 'success' : 'error');
      if (outcome.ok && closeOnSuccess) onDone();
      return outcome.ok;
    } catch (err) {
      addToast(typeof err === 'string' ? err : 'That did not go through', 'error');
      return false;
    } finally {
      setBusy(false);
    }
  };
  return { busy, act };
}

/** The form for one builder, without its frame: the desktop popover and the
 *  phone's Manage channel sheet both draw it. */
export function ManagerForm({
  kind,
  view,
  queue,
  onQueue,
  onOutcome,
  onDone,
}: {
  kind: ManagerPanelKind;
  view: StreamManagerView;
  queue: QueueView | null;
  onQueue: (q: QueueView) => void;
  onOutcome: (o: ActOutcome) => void;
  onDone: () => void;
}) {
  const { busy, act } = useManagerAct(view, onOutcome, onDone);
  return (
    <>
      {kind === 'info' && <InfoEditor view={view} onOutcome={onOutcome} onClose={onDone} framed={false} />}
      {kind === 'raid' && <RaidForm view={view} busy={busy} act={act} />}
      {kind === 'ads' && <AdsForm view={view} busy={busy} act={act} />}
      {kind === 'rewards' && <RewardsQueue view={view} queue={queue} onQueue={onQueue} busy={busy} act={act} />}
      {kind === 'announce' && <AnnounceForm view={view} busy={busy} act={act} />}
    </>
  );
}

export function StreamManagerPanel(props: Props) {
  // Stream info draws its own frame: a footer that stays put under the
  // scrolling fields, and the expanded view in the middle of the screen.
  if (props.kind === 'info') {
    return <InfoEditor view={props.view} onOutcome={props.onOutcome} onClose={props.onClose} framed />;
  }
  return <BuilderPanel {...props} />;
}

function BuilderPanel({ kind, view, queue, onQueue, onOutcome, onClose }: Props) {
  const rootRef = useRef<HTMLDivElement>(null);

  const markInside = useOutsidePress(true, [rootRef], onClose);
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

  const head = TITLES[kind];
  return (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      className="glass-flyout rounded-xl !p-0 absolute bottom-full left-0 right-0 mb-2 z-[60] flex flex-col max-h-[70vh] overflow-hidden"
    >
      <div className="flex items-center gap-2 px-3 py-2.5">
        <span className="flex items-center justify-center text-textSecondary">{head.icon}</span>
        <span className="flex-1 text-sm font-semibold text-textPrimary">{head.title}</span>
        <Tooltip content="Close" side="top">
          <button onClick={onClose} className={`${GLAZE_PILL} grid h-6 w-6 place-items-center`} aria-label="Close">
            <X size={14} />
          </button>
        </Tooltip>
      </div>
      <div className="overflow-y-auto scrollbar-thin px-3 pb-3">
        <ManagerForm kind={kind} view={view} queue={queue} onQueue={onQueue} onOutcome={onOutcome} onDone={onClose} />
      </div>
    </div>
  );
}

type Act = (action: ManagerAction, closeOnSuccess?: boolean) => Promise<boolean>;

interface FormProps {
  view: StreamManagerView;
  busy: boolean;
  act: Act;
}

// ─── stream info ─────────────────────────────────────────────────────────────

/**
 * Unsaved stream info edits, one per channel, for this run of the app. Esc or
 * a press outside closes the editor at once (the app's one outside rule), so
 * the edit waits here and the next open picks it up. It is this window's own
 * form state, never shared or stored, which is why it lives beside the form
 * rather than in Rust.
 */
const keptDrafts = new Map<string, { draft: StreamInfoDraft; base: string }>();

/** What Twitch had when an edit began. A kept edit made against something
 *  else (the title changed meanwhile) starts fresh instead of resuming. */
const infoBase = (info: StreamInfoView | null) =>
  JSON.stringify([info?.title ?? '', info?.category?.id ?? '', info?.tags ?? [], info?.language ?? '']);

const freshDraft = (info: StreamInfoView | null): StreamInfoDraft => ({
  title: info?.title ?? '',
  category: info?.category ?? null,
  tags: info?.tags ?? [],
  language: info?.language ?? '',
});

const boxArt = (c: Category, w: number, h: number) =>
  c.box_art_url.replace('{width}', String(w)).replace('{height}', String(h));

/** How long "Saved" shows before the editor closes itself. */
const SAVED_HOLD_MS = 900;
const KEY_CAP =
  'font-sans rounded-md bg-surface/50 px-1.5 py-[3px] text-[10.5px] font-bold leading-none text-textSecondary ring-1 ring-inset ring-textPrimary/10';
/** Close, expand and Discard: a step quieter than the content and Save. */
const QUIET_PILL = `${GLAZE_PILL} !bg-white/[0.05] hover:!bg-white/[0.1] !text-textSecondary hover:!text-textPrimary`;
/** Saved: the button's own glass tinted green, never a flat fill. */
const SAVED_FILL: CSSProperties = {
  background: 'color-mix(in srgb, var(--color-success) 30%, var(--color-background-tertiary))',
};
const TONE_CLASS = { quiet: 'text-textSecondary', near: 'text-warning', over: 'text-error' } as const;
/** The body's bottom edge fades into the footer instead of cutting. */
const FADE_BOTTOM: CSSProperties = { maskImage: 'linear-gradient(to bottom, black calc(100% - 16px), transparent)' };

type SavePhase = 'idle' | 'saving' | 'saved' | 'failed';

/**
 * Stream info: the title (with an emoji menu and Twitch's count), category,
 * tags and language, and the tile viewers will see. `framed` is the desktop
 * editor over the message box, which can also open large in the middle of
 * the screen; unframed is the phone's Manage channel sheet, which brings its
 * own frame. Rust checks the draft, words the count and every problem, and
 * runs the save; this only lays it out.
 */
function InfoEditor({
  view,
  onOutcome,
  onClose,
  framed,
}: {
  view: StreamManagerView;
  onOutcome: (o: ActOutcome) => void;
  onClose: () => void;
  framed: boolean;
}) {
  const info = view.info;
  const channelId = view.channel_id;
  const [start] = useState(() => {
    const base = infoBase(info);
    const kept = keptDrafts.get(channelId);
    if (kept && kept.base === base) return { draft: kept.draft, base, resumed: true };
    keptDrafts.delete(channelId);
    return { draft: freshDraft(info), base, resumed: false };
  });
  const [draft, setDraftState] = useState<StreamInfoDraft>(start.draft);
  const [resumed, setResumed] = useState(start.resumed);
  const touched = useRef(start.resumed);
  const [check, setCheck] = useState<InfoCheck | null>(null);
  const [phase, setPhase] = useState<SavePhase>('idle');
  const [failLine, setFailLine] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [tagText, setTagText] = useState('');
  const [catQuery, setCatQuery] = useState<string | null>(null);
  const [catResults, setCatResults] = useState<Category[]>([]);
  const [catIndex, setCatIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const emojiButtonRef = useRef<HTMLButtonElement>(null);
  const caretRef = useRef<[number, number] | null>(null);
  const closeTimer = useRef<number | null>(null);
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const perfQuiet = usePerformanceQuiet();
  const instant = motionOff || perfQuiet;
  const big = framed && expanded;

  const setDraft = (change: (d: StreamInfoDraft) => StreamInfoDraft) => {
    touched.current = true;
    setDraftState(change);
    if (phase === 'failed') setPhase('idle');
  };

  // Every change waits in keptDrafts until it is saved or thrown away.
  useEffect(() => {
    if (touched.current) keptDrafts.set(channelId, { draft, base: start.base });
  }, [draft, channelId, start.base]);

  useEffect(() => () => {
    if (closeTimer.current) window.clearTimeout(closeTimer.current);
  }, []);

  // Typing goes straight into the title, caret at its end, on open and when
  // the editor changes size (the field is a new one then).
  useEffect(() => {
    const t = titleRef.current;
    if (!t) return;
    t.focus();
    t.setSelectionRange(t.value.length, t.value.length);
  }, [expanded]);

  const markInside = useOutsidePress(framed, [rootRef], onClose);
  useEffect(() => {
    if (!framed) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [framed, onClose]);

  // Rust checks the draft and builds the preview tile; asked after typing pauses.
  useEffect(() => {
    let alive = true;
    const t = window.setTimeout(() => {
      void checkStreamInfo(view.channel_id, draft)
        .then((c) => alive && setCheck(c))
        .catch(() => {});
    }, 200);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [draft, view.channel_id]);

  useEffect(() => {
    if (catQuery === null || !catQuery.trim()) return;
    let alive = true;
    const t = window.setTimeout(() => {
      void invoke<Category[]>('search_categories', { query: catQuery.trim(), limit: 10 })
        .then((rows) => alive && setCatResults(rows))
        .catch(() => alive && setCatResults([]));
    }, 180);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [catQuery]);

  const problemFor = (field: string) => check?.problems.find((p) => p.field === field)?.line;
  const tagProblem = check?.problems.find((p) => p.field.startsWith('tag'))?.line;

  const addTag = () => {
    const raw = tagText.trim();
    if (!raw) return;
    setDraft((d) => ({ ...d, tags: [...d.tags, raw] }));
    setTagText('');
  };

  const preview = check?.preview;
  const tone = check?.title_tone ?? 'quiet';
  // Past the limit the count says so in the box; the line would repeat it.
  const titleProblem = tone === 'over' ? undefined : problemFor('title');
  const canSave = phase !== 'saving' && phase !== 'saved' && !!check?.ok && check.changed;

  const save = async () => {
    if (!canSave) return;
    setPhase('saving');
    setFailLine(null);
    setPickerOpen(false);
    try {
      const outcome = await managerAct(view.channel_id, view.channel_login, { kind: 'save_info', draft });
      onOutcome(outcome);
      if (!outcome.ok) {
        setFailLine(outcome.message);
        setPhase('failed');
        return;
      }
      keptDrafts.delete(channelId);
      touched.current = false;
      setPhase('saved');
      closeTimer.current = window.setTimeout(onClose, SAVED_HOLD_MS);
    } catch (err) {
      setFailLine(typeof err === 'string' ? err : 'Twitch did not answer.');
      setPhase('failed');
    }
  };

  /** Discard, and Start over on a picked-up edit: back to what Twitch has. */
  const startOver = () => {
    keptDrafts.delete(channelId);
    touched.current = false;
    setDraftState(freshDraft(info));
    setResumed(false);
    setPhase('idle');
    setFailLine(null);
    setTagText('');
    setCatQuery(null);
    titleRef.current?.focus();
  };

  const rememberCaret = () => {
    const t = titleRef.current;
    if (t) caretRef.current = [t.selectionStart, t.selectionEnd];
  };
  const insertEmoji = (emoji: string) => {
    const t = titleRef.current;
    const live = !!t && document.activeElement === t;
    const len = draft.title.length;
    const [a, b] = live ? [t.selectionStart, t.selectionEnd] : (caretRef.current ?? [len, len]);
    const from = Math.min(a, len);
    const title = draft.title.slice(0, from) + emoji + draft.title.slice(Math.min(b, len));
    const caret = from + emoji.length;
    caretRef.current = [caret, caret];
    setDraft((d) => ({ ...d, title }));
    if (live) requestAnimationFrame(() => t.setSelectionRange(caret, caret));
  };

  // Ctrl+Enter saves from anywhere; a plain Enter saves from the title and
  // an empty tag box (they say so with preventDefault).
  const onEditorKey = (e: ReactKeyboardEvent) => {
    if (e.defaultPrevented || e.key !== 'Enter' || !(e.ctrlKey || e.metaKey) || isImeComposing(e)) return;
    e.preventDefault();
    void save();
  };

  // The large view's grid holds one full row of recents.
  const recents = (info?.recent_categories ?? []).slice(0, big ? 5 : undefined);
  const catList = catQuery === null ? [] : catQuery.trim() ? catResults : recents;
  const pickCategory = (c: Category) => {
    setDraft((d) => ({ ...d, category: c }));
    setCatQuery(null);
  };
  const catCols = big ? 5 : 1;

  const avatar = (px: number) =>
    preview?.avatar_url ? (
      <img src={preview.avatar_url} alt="" className="flex-shrink-0 rounded-full" style={{ width: px, height: px }} />
    ) : (
      <span className="flex-shrink-0 rounded-full bg-white/10" style={{ width: px, height: px }} />
    );

  // ── the fields ──

  const titleField = (
    <div>
      <label htmlFor="sn-stream-title" className={`${LABEL} mb-1.5 block`}>
        Title
      </label>
      <div
        className={`glass-input !rounded-xl transition-[border-color] ${
          tone === 'over' ? '!border-error/60' : 'focus-within:!border-accent/40'
        }`}
      >
        {/* Wraps and grows with the title, so a long one reads whole while
            it is edited (SN-264). Still one line of text: Enter saves, and
            pasted line breaks become spaces. */}
        <textarea
          id="sn-stream-title"
          ref={titleRef}
          rows={1}
          value={draft.title}
          onChange={(e) => {
            const title = e.target.value.replace(/\r?\n/g, ' ');
            setDraft((d) => ({ ...d, title }));
          }}
          onSelect={rememberCaret}
          onBlur={rememberCaret}
          onKeyDown={(e) => {
            if (isImeComposing(e)) return;
            if (e.key === 'Escape' && pickerOpen) {
              e.preventDefault();
              e.stopPropagation();
              setPickerOpen(false);
            } else if (e.key === 'Enter') {
              e.preventDefault();
              void save();
            }
          }}
          placeholder="What are you streaming?"
          className={`block w-full resize-none bg-transparent px-3 pt-2.5 pb-1 leading-normal text-textPrimary placeholder-textSecondary outline-none [field-sizing:content] ${
            big
              ? 'min-h-[5.5rem] max-h-[12rem] text-[17px] font-semibold'
              : IS_MOBILE
                ? 'max-h-[8.5rem] text-[15px]'
                : 'max-h-[8.5rem] text-[14px] font-medium'
          }`}
        />
        <div className="flex min-h-[30px] items-center justify-end gap-2 pb-1.5 pl-3 pr-1.5">
          <span className={`text-[11.5px] font-semibold tabular-nums ${TONE_CLASS[tone]}`}>{check?.title_count ?? ''}</span>
          {framed && (
            <Tooltip content="Emoji" side="top">
              <button
                ref={emojiButtonRef}
                type="button"
                aria-label="Emoji"
                aria-expanded={pickerOpen}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => setPickerOpen((o) => !o)}
                className={`${pickerOpen ? `${GLAZE_PILL} !bg-white/[0.16]` : QUIET_PILL} grid h-7 w-7 place-items-center`}
              >
                <SmilePlus size={15} />
              </button>
            </Tooltip>
          )}
        </div>
      </div>
      {titleProblem && <p className="mt-1 text-[11.5px] text-error">{titleProblem}</p>}
    </div>
  );

  const categoryField = (
    <div className="relative">
      <label className={`${LABEL} mb-1.5 block`}>Category</label>
      {catQuery === null ? (
        <button
          type="button"
          onClick={() => {
            setCatQuery('');
            setCatIndex(0);
          }}
          className="glass-input flex w-full items-center gap-2.5 !rounded-xl py-1.5 pl-2 pr-3 text-left transition-colors hover:!border-white/15"
        >
          {draft.category?.box_art_url ? (
            <img src={boxArt(draft.category, 54, 72)} alt="" className="h-9 w-[27px] flex-shrink-0 rounded object-cover" />
          ) : (
            <span className="h-9 w-[27px] flex-shrink-0 rounded bg-white/[0.06]" />
          )}
          <span className={`flex-1 truncate text-sm font-medium ${draft.category ? 'text-textPrimary' : 'text-textSecondary'}`}>
            {draft.category?.name ?? 'Pick a category'}
          </span>
          {big ? (
            <span className="text-xs font-semibold text-textSecondary">Change</span>
          ) : (
            <ChevronDown size={15} className="text-textSecondary" aria-hidden />
          )}
        </button>
      ) : (
        <>
          <div className="glass-input flex items-center gap-2 !rounded-xl px-3 focus-within:!border-accent/40">
            <Search size={15} className="flex-shrink-0 text-textSecondary" aria-hidden />
            <input
              autoFocus
              value={catQuery}
              onChange={(e) => {
                setCatQuery(e.target.value);
                setCatIndex(0);
              }}
              onBlur={() => setCatQuery(null)}
              onKeyDown={(e) => {
                const down = e.key === 'ArrowDown' ? catCols : e.key === 'ArrowRight' && big ? 1 : 0;
                const up = e.key === 'ArrowUp' ? catCols : e.key === 'ArrowLeft' && big ? 1 : 0;
                if (e.key === 'Escape') {
                  e.stopPropagation();
                  setCatQuery(null);
                } else if (down || up) {
                  e.preventDefault();
                  setCatIndex((i) => Math.max(0, Math.min(catList.length - 1, i + down - up)));
                } else if (e.key === 'Enter') {
                  e.preventDefault();
                  const c = catList[catIndex];
                  if (c) pickCategory(c);
                }
              }}
              placeholder="Search categories"
              className="h-12 min-w-0 flex-1 bg-transparent text-sm text-textPrimary placeholder-textSecondary outline-none"
            />
          </div>
        </>
      )}
    </div>
  );

  // The results take the place of the fields under Category while you
  // search (those stay in the layout, unseen), so the box you clicked stays
  // put and nothing shows through or gets cut off.
  const searching = catQuery !== null && catList.length > 0;
  const catResultsView = searching && (
    <div className="absolute inset-0 z-10 overflow-y-auto scrollbar-thin">
              {!catQuery?.trim() && <p className={`${LABEL} mb-1.5`}>Your recent categories</p>}
              {big ? (
                <div className="grid grid-cols-5 gap-2.5">
                  {catList.map((c, i) => (
                    <button
                      key={c.id}
                      type="button"
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => pickCategory(c)}
                      className="group min-w-0 text-center"
                    >
                      <img
                        src={boxArt(c, 144, 192)}
                        alt=""
                        className={`aspect-[3/4] w-full rounded-md object-cover transition-shadow ${
                          draft.category?.id === c.id
                            ? 'ring-2 ring-white/75'
                            : i === catIndex
                              ? 'ring-2 ring-white/30'
                              : 'group-hover:ring-2 group-hover:ring-white/20'
                        }`}
                      />
                      <span className="mt-1 block truncate text-[11.5px] font-semibold text-textSecondary">{c.name}</span>
                    </button>
                  ))}
                </div>
              ) : (
                <div>
                  {catList.map((c, i) => (
                    <button
                      key={c.id}
                      type="button"
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => pickCategory(c)}
                      className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-white/[0.06] ${
                        i === catIndex ? 'bg-white/[0.07]' : ''
                      }`}
                    >
                      <img src={boxArt(c, 54, 72)} alt="" className="h-8 w-6 flex-shrink-0 rounded-sm object-cover" />
                      <span className="flex-1 truncate text-sm text-textPrimary">{c.name}</span>
                      {draft.category?.id === c.id && <Check size={14} className="text-textSecondary" aria-hidden />}
                    </button>
                  ))}
                </div>
              )}
    </div>
  );
  /** The fields under Category, with the search results over them. */
  const underCategory = (fields: ReactNode) => (
    <div className={`relative ${searching && big ? 'min-h-[196px]' : ''}`}>
      <div className={`${big ? 'space-y-4' : 'space-y-3.5'} ${searching ? 'invisible' : ''}`}>{fields}</div>
      {catResultsView}
    </div>
  );

  const tagsField = (
    <div>
      <div className="mb-1.5 flex items-baseline justify-between">
        <label className={LABEL}>Tags</label>
        <span className="text-[11px] font-semibold tabular-nums text-textSecondary/80">{check?.tags_count ?? ''}</span>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        {draft.tags.map((t, i) => (
          <span
            key={`${t}-${i}`}
            className={`${GLAZE_PILL} flex h-[26px] items-center gap-1 pl-2.5 pr-1 text-[12.5px] !font-medium ${
              problemFor(`tag:${i}`) ? '!bg-error/15 !text-error' : ''
            }`}
          >
            {t}
            <button
              type="button"
              aria-label={`Remove ${t}`}
              onClick={() => setDraft((d) => ({ ...d, tags: d.tags.filter((_, j) => j !== i) }))}
              className="grid h-5 w-5 place-items-center rounded-full text-textSecondary transition-colors hover:bg-white/10 hover:text-textPrimary"
            >
              <X size={11} />
            </button>
          </span>
        ))}
        {draft.tags.length < view.limits.tags && (
          <input
            value={tagText}
            onChange={(e) => setTagText(e.target.value)}
            onKeyDown={(e) => {
              if (isImeComposing(e)) return;
              if (e.key === 'Enter') {
                e.preventDefault();
                if (tagText.trim()) addTag();
                else void save();
              } else if (e.key === ',') {
                e.preventDefault();
                addTag();
              } else if (e.key === 'Backspace' && !tagText && draft.tags.length) {
                setDraft((d) => ({ ...d, tags: d.tags.slice(0, -1) }));
              }
            }}
            onBlur={addTag}
            placeholder="+ Add a tag"
            className="h-[26px] min-w-[4.5rem] flex-1 bg-transparent px-1 text-[12.5px] text-textPrimary placeholder-textSecondary outline-none"
          />
        )}
      </div>
      {tagProblem && <p className="mt-1 text-[11.5px] text-error">{tagProblem}</p>}
    </div>
  );

  const languageRow = (
    <div>
      <div className="flex items-center justify-between gap-3">
        <label htmlFor="sn-stream-language" className="text-[13px] text-textSecondary">
          Language
        </label>
        <div className="relative flex items-center">
          <select
            id="sn-stream-language"
            value={draft.language}
            onChange={(e) => setDraft((d) => ({ ...d, language: e.target.value }))}
            className="cursor-pointer appearance-none bg-transparent pr-5 text-right text-[13px] font-semibold text-textPrimary outline-none"
          >
            {(info?.languages ?? []).map((l) => (
              <option key={l.code} value={l.code} style={{ background: 'var(--color-background-tertiary)' }}>
                {l.name}
              </option>
            ))}
          </select>
          <ChevronDown size={14} className="pointer-events-none absolute right-0 text-textSecondary" aria-hidden />
        </div>
      </div>
      {problemFor('language') && <p className="mt-1 text-[11.5px] text-error">{problemFor('language')}</p>}
    </div>
  );

  const live = (
    <span className="absolute left-1.5 top-1.5 rounded bg-[#e5484d] px-1 py-px text-[10px] font-extrabold tracking-wide text-white">LIVE</span>
  );

  const previewCompact = preview && (
    <div>
      <p className={`${LABEL} mb-1.5`}>What viewers see</p>
      <div className="flex gap-2.5 rounded-xl bg-surface/25 p-2 ring-1 ring-inset ring-textPrimary/[0.05]">
        <div className="relative aspect-video w-[112px] flex-shrink-0 overflow-hidden rounded-lg bg-white/[0.05]">
          {preview.thumbnail_url && (
            <>
              <img src={preview.thumbnail_url} alt="" className="h-full w-full object-cover" />
              {live}
            </>
          )}
        </div>
        <div className="min-w-0 flex-1">
          <p className="line-clamp-2 text-[13.5px] font-semibold leading-snug text-textPrimary">{preview.title || 'Your title'}</p>
          <p className="mt-0.5 truncate text-[12.5px] text-textSecondary">
            {[preview.channel_name, preview.category_name].filter(Boolean).join(' · ')}
          </p>
        </div>
      </div>
    </div>
  );

  const previewLarge = preview && (
    <div className="min-w-0">
      <p className={`${LABEL} mb-1.5`}>What viewers see</p>
      <div className="relative aspect-video w-full overflow-hidden rounded-lg bg-white/[0.05]">
        {preview.thumbnail_url && (
          <>
            <img src={preview.thumbnail_url} alt="" className="h-full w-full object-cover" />
            {live}
          </>
        )}
      </div>
      <div className="mt-2.5 flex gap-2.5">
        {avatar(30)}
        <div className="min-w-0 flex-1">
          <p className="line-clamp-2 text-[13.5px] font-semibold leading-snug text-textPrimary">{preview.title || 'Your title'}</p>
          <p className="mt-0.5 truncate text-[12.5px] text-textSecondary">{preview.channel_name}</p>
          {preview.category_name && <p className="truncate text-[12.5px] text-textSecondary">{preview.category_name}</p>}
          {preview.tags.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {preview.tags.slice(0, 4).map((t) => (
                <span key={t} className="rounded-full bg-surface/60 px-2 py-px text-[11px] text-textSecondary">
                  {t}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
      <p className="mb-1.5 mt-4 text-[11.5px] font-semibold text-textSecondary/80">In the sidebar</p>
      <div className="flex items-center gap-2.5 rounded-xl bg-surface/25 p-2 ring-1 ring-inset ring-textPrimary/[0.05]">
        {avatar(28)}
        <div className="min-w-0 flex-1">
          <p className="truncate text-[12.5px] font-semibold text-textPrimary">{preview.channel_name}</p>
          <p className="truncate text-[12px] text-textSecondary">{preview.title || 'Your title'}</p>
        </div>
      </div>
    </div>
  );

  const resumeLine = resumed && check?.changed && phase !== 'saved' && (
    <div className="flex items-center gap-2 rounded-lg bg-surface/30 py-1.5 pl-2.5 pr-1.5 text-[12.5px] text-textSecondary ring-1 ring-inset ring-textPrimary/[0.06]">
      <Undo2 size={14} className="flex-shrink-0" aria-hidden />
      <span className="flex-1">Picking up your unsaved edit</span>
      <button type="button" onClick={startOver} className={`${GLAZE_PILL} h-6 px-2.5 text-[12px]`}>
        Start over
      </button>
    </div>
  );

  const failBox = phase === 'failed' && failLine && (
    <div role="alert" className="flex items-start gap-2 rounded-lg bg-error/10 px-3 py-2 text-[12.5px] leading-snug text-error ring-1 ring-inset ring-error/25">
      <CircleAlert size={15} className="mt-px flex-shrink-0" aria-hidden />
      <span>
        Couldn't save. {failLine} Your changes are still here.
      </span>
    </div>
  );

  const saveLabel =
    phase === 'saving' ? 'Saving' : phase === 'saved' ? 'Saved' : phase === 'failed' ? 'Try again' : 'Save';
  const footer = (
    <div className={`flex items-center gap-2 ${framed ? `border-t border-textPrimary/[0.08] ${big ? 'px-5 py-3' : 'px-3.5 py-2.5'}` : 'pt-1'}`}>
      {framed ? (
        <span className="flex flex-1 items-center gap-1.5 text-[11.5px] text-textSecondary/80">
          <kbd className={KEY_CAP}>Enter</kbd>
          save
          <kbd className={`${KEY_CAP} ml-1`}>Esc</kbd>
          close
        </span>
      ) : (
        <span className="flex-1" />
      )}
      {check?.changed && phase !== 'saved' && (
        <button
          type="button"
          onClick={startOver}
          disabled={phase === 'saving'}
          className={`${QUIET_PILL} ${framed ? 'h-[34px] px-4 text-[13.5px]' : `${BIG} px-5`}`}
        >
          Discard
        </button>
      )}
      <button
        type="button"
        onClick={() => void save()}
        disabled={phase !== 'saved' && !canSave}
        aria-live="polite"
        className={`${ACCENT_BUTTON} flex items-center justify-center gap-1.5 disabled:cursor-not-allowed ${
          framed ? `h-[34px] min-w-[104px] ${big ? 'px-6' : 'px-5'} !text-[13.5px]` : `${BIG} flex-1`
        }`}
        style={phase === 'saved' ? SAVED_FILL : ACCENT_FILL}
      >
        {phase === 'saving' && <LoaderCircle size={14} className="animate-spin" aria-hidden />}
        {phase === 'saved' && <Check size={15} aria-hidden />}
        {saveLabel}
      </button>
    </div>
  );

  const picker = framed && pickerOpen && info && (
    <TitleEmojiPicker
      anchorRef={emojiButtonRef}
      recent={info.title_emoji}
      recentHeading={info.title_emoji_heading}
      onPick={insertEmoji}
      onClose={() => setPickerOpen(false)}
    />
  );

  // ── the frames ──

  if (!framed) {
    return (
      <div className="space-y-3.5" onKeyDown={onEditorKey}>
        {resumeLine}
        {titleField}
        {categoryField}
        {underCategory(
          <>
            {tagsField}
            {languageRow}
            {previewCompact}
          </>,
        )}
        {failBox}
        {footer}
      </div>
    );
  }

  const iconButton = (label: string, onClick: () => void, icon: ReactNode, size: string) => (
    <Tooltip content={label} side="top">
      <button type="button" onClick={onClick} aria-label={label} className={`${QUIET_PILL} grid ${size} place-items-center`}>
        {icon}
      </button>
    </Tooltip>
  );

  if (big) {
    const ease = instant ? { duration: 0 } : { duration: 0.2, ease: [0.2, 0.8, 0.2, 1] as const };
    return createPortal(
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={instant ? { duration: 0 } : { duration: 0.15 }}
        // Pinned from the top, so a list opening inside grows it downward
        // instead of re-centering it under the pointer.
        className="fixed inset-0 z-[calc(var(--sn-z-over-player-lift,0)_+_200)] flex items-start justify-center bg-black/50 px-6 pb-6 pt-[max(24px,calc(50vh-300px))]"
      >
        <motion.div
          ref={rootRef}
          onPointerDownCapture={markInside}
          onKeyDown={onEditorKey}
          role="dialog"
          aria-modal
          aria-label="Stream info"
          initial={{ opacity: 0, y: 8, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={ease}
          className="glass-flyout flex max-h-[min(760px,92vh)] w-[min(780px,96vw)] flex-col overflow-hidden rounded-2xl !p-0"
        >
          <div className="flex items-center gap-3 px-5 pb-3 pt-4">
            {avatar(34)}
            <div className="min-w-0 flex-1">
              <p className="text-[16px] font-semibold leading-tight text-textPrimary">Stream info</p>
              <p className="truncate text-[12.5px] text-textSecondary">
                {[preview?.channel_name || view.channel_login, view.role_line].join(' · ')}
              </p>
            </div>
            {/* On the title's line, not centered on title and subtitle. */}
            <div className="-mt-[5px] flex gap-2 self-start">
              {iconButton('Back to the panel', () => setExpanded(false), <Minimize2 size={14} />, 'h-[30px] w-[30px]')}
              {iconButton('Close', onClose, <X size={15} />, 'h-[30px] w-[30px]')}
            </div>
          </div>
          {resumeLine && <div className="px-5 pb-3">{resumeLine}</div>}
          <div
            className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_260px] gap-6 overflow-y-auto scrollbar-thin px-5 pb-5 pt-0.5"
            style={FADE_BOTTOM}
          >
            <div className="min-w-0 space-y-4">
              {titleField}
              {categoryField}
              {underCategory(
                <>
                  {tagsField}
                  {languageRow}
                </>,
              )}
            </div>
            {previewLarge}
          </div>
          {failBox && <div className="px-5 pb-3">{failBox}</div>}
          {footer}
          {picker}
        </motion.div>
      </motion.div>,
      document.body,
    );
  }

  return (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      onKeyDown={onEditorKey}
      className="glass-flyout rounded-xl !p-0 absolute bottom-full left-0 right-0 mb-2 z-[60] flex max-h-[70vh] flex-col overflow-hidden"
    >
      <div className="flex items-center gap-2 px-3.5 pb-2 pt-3">
        <Pencil size={14} className="text-textSecondary" aria-hidden />
        <span className="flex-1 text-sm font-semibold text-textPrimary">Stream info</span>
        {iconButton('Open larger', () => setExpanded(true), <Maximize2 size={13} />, 'h-[26px] w-[26px]')}
        {iconButton('Close', onClose, <X size={14} />, 'h-[26px] w-[26px]')}
      </div>
      {resumeLine && <div className="px-3.5 pb-2">{resumeLine}</div>}
      <div className="min-h-0 flex-1 space-y-3.5 overflow-y-auto scrollbar-thin px-3.5 pb-4 pt-0.5" style={FADE_BOTTOM}>
        {titleField}
        {categoryField}
        {underCategory(
          <>
            {tagsField}
            {languageRow}
            {previewCompact}
          </>,
        )}
      </div>
      {failBox && <div className="px-3.5 pb-2.5">{failBox}</div>}
      {footer}
      {picker}
    </div>
  );
}

// ─── raid ────────────────────────────────────────────────────────────────────

function RaidForm({ view, busy, act }: FormProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<RaidTarget[]>([]);
  const [now, setNow] = useState(() => Date.now());
  const raid = view.raid;

  useEffect(() => {
    if (!raid) return;
    const t = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(t);
  }, [raid]);

  useEffect(() => {
    if (raid || !query.trim()) return;
    let alive = true;
    const t = window.setTimeout(() => {
      void searchRaidTargets(query, view.channel_id)
        .then((r) => alive && setResults(r))
        .catch(() => alive && setResults([]));
    }, 220);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [query, raid, view.channel_id]);

  if (raid) {
    return (
      <div className="space-y-3">
        <div className="text-center pt-1">
          <p className="text-sm text-textSecondary">Raiding {raid.target_name}</p>
          <p className="text-3xl font-bold tabular-nums text-textPrimary mt-1">{countdown(raid.deadline_ms, now)}</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => void act({ kind: 'cancel_raid' })}
            disabled={busy}
            className={`${GLAZE_PILL} flex-1 ${BIG}`}
          >
            Cancel raid
          </button>
          <button
            onClick={() => void act({ kind: 'raid_now' }, true)}
            disabled={busy}
            className={`${ACCENT_BUTTON} flex-1 ${BIG}`}
            style={ACCENT_FILL}
          >
            Raid now
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <input
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search for a channel"
        className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary"
      />
      {(query.trim() ? results : []).map((t) => (
        <button
          key={t.id}
          disabled={busy || !t.live}
          onClick={() => void act({ kind: 'raid', target_id: t.id, target_login: t.login, target_name: t.name })}
          className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left hover:bg-white/[0.06] transition-colors disabled:opacity-50 disabled:hover:bg-transparent"
        >
          {t.avatar_url ? (
            <img src={t.avatar_url} alt="" className="w-7 h-7 rounded-full flex-shrink-0" />
          ) : (
            <span className="w-7 h-7 rounded-full bg-white/10 flex-shrink-0" />
          )}
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-semibold text-textPrimary truncate">{t.name}</span>
            <span className="block text-xs text-textSecondary truncate">{t.line}</span>
          </span>
        </button>
      ))}
    </div>
  );
}

// ─── ads ─────────────────────────────────────────────────────────────────────

function AdsForm({ view, busy, act }: FormProps) {
  const ads = view.ads;
  const [seconds, setSeconds] = useState(ads?.lengths[1]?.value ?? 60);
  const detail = view.sections.find((s) => s.id === 'ads')?.detail;
  if (!ads) return null;
  const s = ads.schedule;
  return (
    <div className="space-y-3">
      {(detail || s?.preroll_line || ads.schedule_hint) && (
        <div className="space-y-0.5">
          {detail && <p className="text-sm text-textPrimary">{detail}</p>}
          {s?.preroll_line && <p className="text-xs text-textSecondary">{s.preroll_line}</p>}
          {ads.schedule_hint && <p className="text-xs text-textSecondary">{ads.schedule_hint}</p>}
        </div>
      )}
      <div>
        <label className={`${LABEL} block mb-1`}>Length</label>
        <div className="flex gap-1">
          {ads.lengths.map((l) => (
            <button
              key={l.value}
              onClick={() => setSeconds(l.value)}
              className={`flex-1 h-7 ${seconds === l.value ? `${ACCENT_BUTTON} !text-xs` : `${GLAZE_PILL} text-xs`}`}
              style={seconds === l.value ? ACCENT_FILL : undefined}
            >
              {l.label}
            </button>
          ))}
        </div>
      </div>
      <div className="flex gap-2">
        {s && s.snoozes_left > 0 && (
          <button
            onClick={() => void act({ kind: 'snooze_ad' })}
            disabled={busy}
            className={`${GLAZE_PILL} flex-1 ${BIG}`}
          >
            Snooze next ad
          </button>
        )}
        <button
          onClick={() => void act({ kind: 'run_ad', seconds }, true)}
          disabled={busy}
          className={`${ACCENT_BUTTON} flex-1 ${BIG}`}
          style={ACCENT_FILL}
        >
          {busy ? 'Starting…' : 'Run ad break'}
        </button>
      </div>
    </div>
  );
}

// ─── rewards queue ───────────────────────────────────────────────────────────

function RewardsQueue({
  view,
  queue,
  onQueue,
  busy,
  act,
}: FormProps & { queue: QueueView | null; onQueue: (q: QueueView) => void }) {
  const [error, setError] = useState<string | null>(null);
  const channelId = view.channel_id;

  useEffect(() => {
    let alive = true;
    void getRewardQueue(channelId)
      .then((q) => {
        if (!alive) return;
        onQueue(q);
        setError(null);
      })
      .catch((e) => alive && setError(typeof e === 'string' ? e : 'Could not read the queue'));
    return () => {
      alive = false;
    };
  }, [channelId, onQueue]);

  const groups = useMemo(() => queue?.groups ?? [], [queue]);
  if (error) return <p className="text-sm text-textSecondary py-2">{error}</p>;
  if (!queue) return <p className="text-sm text-textSecondary py-2">Loading…</p>;
  if (queue.empty_line) return <p className="text-sm text-textSecondary py-2">{queue.empty_line}</p>;

  return (
    <div className="space-y-3">
      {groups.map((g) => (
        <div key={g.reward_id}>
          <div className="flex items-center gap-2 mb-1">
            <span className="flex-1 min-w-0 text-xs font-semibold text-textSecondary truncate">{g.heading}</span>
            <button
              onClick={() => void act({ kind: 'reward', reward_id: g.reward_id, paused: !g.paused })}
              disabled={busy}
              className={`${GLAZE_PILL} sm-btn sm-head h-5 px-2.5 text-[11px]`}
            >
              {g.paused ? 'Resume' : 'Pause'}
            </button>
            {g.rows.length > 1 && (
              <button
                onClick={() => void act({ kind: 'reward_queue', reward_id: g.reward_id, fulfill: true })}
                disabled={busy}
                className={`${GLAZE_PILL} sm-btn sm-head h-5 px-2.5 text-[11px]`}
              >
                Complete all
              </button>
            )}
          </div>
          <div className="rounded-lg bg-white/5 divide-y divide-white/5">
            {g.rows.map((r) => (
              <div key={r.id} className={`sm-row flex gap-2.5 px-2.5 py-2 ${r.input ? 'items-start' : 'items-center'}`}>
                {r.avatar_url ? (
                  <img src={r.avatar_url} alt="" className={`w-6 h-6 rounded-full flex-shrink-0 ${r.input ? 'mt-0.5' : ''}`} />
                ) : (
                  <span className={`w-6 h-6 rounded-full bg-white/10 flex-shrink-0 ${r.input ? 'mt-0.5' : ''}`} />
                )}
                <div className="min-w-0 flex-1">
                  <p className="text-[11px] text-textSecondary truncate">{r.line}</p>
                  {r.input && <p className="text-sm text-textPrimary break-words">{r.input}</p>}
                </div>
                <div className="sm-acts flex gap-1 flex-shrink-0">
                  <button
                    onClick={() => void act({ kind: 'redemptions', ids: [r.id], fulfill: true })}
                    disabled={busy}
                    className={`${ACCENT_BUTTON} sm-btn h-6 px-2.5 ${IS_MOBILE ? '' : '!text-[11px]'}`}
                    style={ACCENT_FILL}
                  >
                    Complete
                  </button>
                  <Tooltip content="Refunds the points" side="top">
                    <button
                      onClick={() => void act({ kind: 'redemptions', ids: [r.id], fulfill: false })}
                      disabled={busy}
                      className={`${GLAZE_PILL} sm-btn h-6 px-2.5 text-[11px]`}
                    >
                      Reject
                    </button>
                  </Tooltip>
                </div>
              </div>
            ))}
          </div>
        </div>
      ))}
      {queue.more && <p className="text-xs text-textSecondary">More are waiting; they show here as these clear.</p>}
    </div>
  );
}

// ─── announce ────────────────────────────────────────────────────────────────

const ANNOUNCE_DOT: Record<string, string> = {
  primary: 'bg-accent',
  blue: 'bg-blue-500',
  green: 'bg-green-500',
  orange: 'bg-orange-500',
  purple: 'bg-purple-500',
};

function AnnounceForm({ view, busy, act }: FormProps) {
  const [text, setText] = useState('');
  const [color, setColor] = useState('primary');
  const colors = view.chat?.announce_colors ?? [];
  const send = async () => {
    if (await act({ kind: 'announce', text, color }, true)) setText('');
  };
  return (
    <div className="space-y-3">
      <textarea
        autoFocus
        value={text}
        rows={3}
        maxLength={500}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !isImeComposing(e)) {
            e.preventDefault();
            void send();
          }
        }}
        placeholder="Your announcement"
        className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary resize-none"
      />
      <div className="flex items-center gap-1.5">
        {colors.map((c) => (
          <Tooltip key={c.id} content={c.label} side="top">
            <button
              onClick={() => setColor(c.id)}
              aria-label={c.label}
              aria-pressed={color === c.id}
              className={`w-6 h-6 rounded-full flex items-center justify-center transition-shadow ${
                color === c.id ? 'ring-2 ring-white/70' : ''
              }`}
            >
              <span className={`w-4 h-4 rounded-full ${ANNOUNCE_DOT[c.id] ?? 'bg-white/40'}`} />
            </button>
          </Tooltip>
        ))}
        <button
          onClick={() => void send()}
          disabled={busy || !text.trim()}
          className={`${ACCENT_BUTTON} ml-auto px-4 ${IS_MOBILE ? 'min-h-[44px] !text-[15px]' : 'h-8 !text-sm'}`}
          style={ACCENT_FILL}
        >
          Announce
        </button>
      </div>
    </div>
  );
}

export default StreamManagerPanel;
