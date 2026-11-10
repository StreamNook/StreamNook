import { useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowLeft,
  BarChart3,
  Check,
  ChevronDown,
  LoaderCircle,
  Megaphone,
  Pencil,
  Search,
  Send,
  SmilePlus,
  Trophy,
  Tv,
  Undo2,
  X,
} from 'lucide-react';
import { TitleEmojiPicker } from './TitleEmojiPicker';
import { Tooltip } from '../ui/Tooltip';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { isImeComposing } from '../../utils/imeComposing';
import { IS_MOBILE } from '../../utils/platform';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import {
  checkStreamInfo,
  countdown,
  getRewardQueue,
  managerAct,
  searchRaidTargets,
  type Category,
  type InfoCheck,
  type QueueGroup,
  type QueueRow,
  type QueueView,
  type RaidTarget,
  type StreamInfoDraft,
  type StreamInfoView,
  type StreamManagerView,
} from '../../services/streamManager';
import {
  clearToolDraft,
  codePoints,
  getTool,
  isShowing,
  raidStart,
  readTool,
  runTool,
  setToolDraft,
  toolKey,
  useManagerStore,
  useTool,
  withHeld,
  type LeavingRow,
  type PanelKind,
  type RaidDraft,
} from './managerStore';
import {
  FIELD,
  FooterRow,
  FormShell,
  GroupCard,
  LABEL,
  MainButton,
  QUIET_PILL,
  ResultLine,
  RowButton,
  SecondaryButton,
  Segmented,
  TONE_CLASS,
  easeFor,
  useCloseSoon,
  useInstant,
  type Layout,
} from './managerKit';

/**
 * The stream manager's forms: stream info, raid, ad break, rewards queue and
 * announcements, plus the running poll and prediction rows. Each draws in
 * three layouts: the panel over the message box, a page of the Manage channel
 * window, and the phone's Manage channel sheet. Everything here draws Rust's
 * view (services/stream_manager.rs) and every write goes through its one
 * door; drafts and results live in managerStore, so they survive the panel
 * closing, opening larger and collapsing back.
 */

export type ManagerPanelKind = PanelKind;

export const TITLES: Record<ManagerPanelKind, { title: string; icon: ReactNode }> = {
  info: { title: 'Stream info', icon: <Pencil size={14} /> },
  raid: { title: 'Raid a channel', icon: <Send size={14} /> },
  ads: { title: 'Ad break', icon: <Tv size={14} /> },
  rewards: { title: 'Rewards queue', icon: <ChannelPointsIcon size={14} /> },
  announce: { title: 'Announce', icon: <Megaphone size={14} /> },
};

interface FormProps {
  view: StreamManagerView;
  layout: Layout;
  onClose: () => void;
  /** Open this tool in the Manage channel window (the panel only). */
  onExpand?: () => void;
  /** Opened from the other size with what was typed: no "Picking up" line. */
  handedOff?: boolean;
}

interface QueueProps {
  queue: QueueView | null;
  onQueue: (channelId: string, q: QueueView) => void;
}

/** One form in the layout asked for. */
export function ManagerForm({ kind, ...props }: FormProps & QueueProps & { kind: ManagerPanelKind }) {
  switch (kind) {
    case 'info':
      return <InfoEditor {...props} />;
    case 'raid':
      return <RaidForm {...props} />;
    case 'ads':
      return <AdsForm {...props} />;
    case 'rewards':
      return <RewardsQueue {...props} />;
    case 'announce':
      return <AnnounceForm {...props} />;
  }
}

/** The desktop panel over the message box. */
export function StreamManagerPanel(props: Omit<FormProps, 'layout'> & QueueProps & { kind: ManagerPanelKind }) {
  return <ManagerForm {...props} layout="panel" />;
}

const frameOf = (kind: ManagerPanelKind, p: FormProps) => ({
  title: TITLES[kind].title,
  icon: TITLES[kind].icon,
  onClose: p.onClose,
  onExpand: p.onExpand,
});

/** Now, ticking every `ms` while `ms` > 0. */
function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (ms <= 0) return;
    // Right away too, so a countdown that starts on an open form never
    // shows the time since the form opened.
    const first = window.setTimeout(() => setNow(Date.now()), 0);
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(t);
    };
  }, [ms]);
  return now;
}

const failWords = (e: { phase: string; line: string | null }) => (e.phase === 'failed' ? e.line : null);

// ─── stream info ─────────────────────────────────────────────────────────────

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

/** Stream info's draft in the store: the fields and a half-typed tag. */
interface InfoHeld {
  draft: StreamInfoDraft;
  tagText: string;
}

const boxArt = (c: Category, w: number, h: number) =>
  c.box_art_url.replace('{width}', String(w)).replace('{height}', String(h));

/**
 * Stream info: the title (with an emoji menu and Twitch's count), category,
 * tags and language, and the tile viewers will see. Rust checks the draft,
 * words the count and every problem, and runs the save; this only lays it out.
 */
function InfoEditor({ view, layout, onClose, onExpand, handedOff = false }: FormProps) {
  const info = view.info;
  const channelId = view.channel_id;
  const key = toolKey(channelId, 'info');
  const entry = useTool(key);
  const base = infoBase(info);
  const fresh = useMemo(() => freshDraft(info), [info]);
  // Decided once, when this editor opens: a kept edit made against other info
  // than Twitch has now starts over; one handed over from the other size
  // carries on without saying so.
  const [opened] = useState(() => {
    const kept = getTool(key);
    const has = kept.draft !== undefined;
    const fits = has && kept.base === base;
    return { stale: has && !fits ? kept.draft : undefined, resumed: fits && !handedOff };
  });
  useEffect(() => {
    if (opened.stale !== undefined) clearToolDraft(key);
  }, [key, opened.stale]);
  // A stale kept edit is never shown, even for the moment before it is cleared.
  const held = entry.draft !== undefined && entry.draft === opened.stale ? undefined : (entry.draft as InfoHeld | undefined);
  const draft = held?.draft ?? fresh;
  const tagText = held?.tagText ?? '';
  const [resumed, setResumed] = useState(opened.resumed);
  const [check, setCheck] = useState<InfoCheck | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [catQuery, setCatQuery] = useState<string | null>(null);
  const [catResults, setCatResults] = useState<Category[]>([]);
  const [catIndex, setCatIndex] = useState(0);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const emojiButtonRef = useRef<HTMLButtonElement>(null);
  const caretRef = useRef<[number, number] | null>(null);
  const closeSoon = useCloseSoon(onClose);
  const big = layout === 'window';
  const sheet = layout === 'sheet';
  const phase = entry.phase;

  /** Every change waits in the store until it is saved or thrown away. */
  const put = (next: InfoHeld) => {
    setToolDraft(key, next, held ? entry.base : base);
  };
  const setDraft = (change: (d: StreamInfoDraft) => StreamInfoDraft) => put({ draft: change(draft), tagText });
  const setTagText = (t: string) => put({ draft, tagText: t });

  // Typing goes straight into the title, caret at its end. Not on the phone,
  // where the keyboard would cover the form the moment it opens.
  useEffect(() => {
    if (sheet) return;
    const t = titleRef.current;
    if (!t) return;
    t.focus();
    t.setSelectionRange(t.value.length, t.value.length);
  }, [sheet]);

  // Rust checks the draft and builds the preview tile; asked after typing pauses.
  useEffect(() => {
    let alive = true;
    const t = window.setTimeout(() => {
      void checkStreamInfo(channelId, draft)
        .then((c) => alive && setCheck(c))
        .catch(() => {});
    }, 200);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [draft, channelId]);

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
    put({ draft: { ...draft, tags: [...draft.tags, raw] }, tagText: '' });
  };

  const preview = check?.preview;
  const tone = check?.title_tone ?? 'quiet';
  // Past the limit the count says so in the box; the line would repeat it.
  const titleProblem = tone === 'over' ? undefined : problemFor('title');
  const canSave = phase !== 'working' && phase !== 'done' && !!check?.ok && check.changed;

  const save = async () => {
    if (!canSave) return;
    setPickerOpen(false);
    const sent = draft;
    const ok = await runTool(key, () => managerAct(channelId, view.channel_login, { kind: 'save_info', draft: sent }), {
      clearDraft: true,
    });
    if (ok && layout !== 'window') closeSoon();
  };

  /** Discard, and Start over on a picked-up edit: back to what Twitch has. */
  const startOver = () => {
    clearToolDraft(key);
    setResumed(false);
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
      <span className="flex-shrink-0 rounded-full bg-textPrimary/10" style={{ width: px, height: px }} />
    );

  // ── the fields ──

  const titleField = (
    <div>
      <label htmlFor={`sn-stream-title-${layout}`} className={`${LABEL} mb-1.5 block`}>
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
          id={`sn-stream-title-${layout}`}
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
          {!sheet && (
            <Tooltip content="Emoji" side="top">
              <button
                ref={emojiButtonRef}
                type="button"
                aria-label="Emoji"
                aria-expanded={pickerOpen}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => setPickerOpen((o) => !o)}
                className={`${pickerOpen ? `${GLAZE_PILL} !bg-textPrimary/[0.16]` : QUIET_PILL} grid h-7 w-7 place-items-center`}
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
          className="glass-input flex w-full items-center gap-2.5 !rounded-xl py-1.5 pl-2 pr-3 text-left transition-colors hover:!border-textPrimary/15"
        >
          {draft.category?.box_art_url ? (
            <img src={boxArt(draft.category, 54, 72)} alt="" className="h-9 w-[27px] flex-shrink-0 rounded object-cover" />
          ) : (
            <span className="h-9 w-[27px] flex-shrink-0 rounded bg-textPrimary/[0.06]" />
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
                e.preventDefault();
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
                    ? 'ring-2 ring-textPrimary/75'
                    : i === catIndex
                      ? 'ring-2 ring-textPrimary/30'
                      : 'group-hover:ring-2 group-hover:ring-textPrimary/20'
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
              className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-textPrimary/[0.06] ${
                i === catIndex ? 'bg-textPrimary/[0.07]' : ''
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
              className="grid h-5 w-5 place-items-center rounded-full text-textSecondary transition-colors hover:bg-textPrimary/10 hover:text-textPrimary"
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
        <label htmlFor={`sn-stream-language-${layout}`} className="text-[13px] text-textSecondary">
          Language
        </label>
        <div className="relative flex items-center">
          <select
            id={`sn-stream-language-${layout}`}
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

  const liveTag = (
    <span className="absolute left-1.5 top-1.5 rounded bg-[#e5484d] px-1 py-px text-[10px] font-extrabold tracking-wide text-white">LIVE</span>
  );

  const previewCompact = preview && (
    <div>
      <p className={`${LABEL} mb-1.5`}>What viewers see</p>
      <div className="flex gap-2.5 rounded-xl bg-surface/25 p-2 ring-1 ring-inset ring-textPrimary/[0.05]">
        <div className="relative aspect-video w-[112px] flex-shrink-0 overflow-hidden rounded-lg bg-textPrimary/[0.05]">
          {preview.thumbnail_url && (
            <>
              <img src={preview.thumbnail_url} alt="" className="h-full w-full object-cover" />
              {liveTag}
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
      <div className="relative aspect-video w-full overflow-hidden rounded-lg bg-textPrimary/[0.05]">
        {preview.thumbnail_url && (
          <>
            <img src={preview.thumbnail_url} alt="" className="h-full w-full object-cover" />
            {liveTag}
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

  const resumeLine = resumed && check?.changed && phase !== 'done' && (
    <div className="flex items-center gap-2 rounded-lg bg-surface/30 py-1.5 pl-2.5 pr-1.5 text-[12.5px] text-textSecondary ring-1 ring-inset ring-textPrimary/[0.06]">
      <Undo2 size={14} className="flex-shrink-0" aria-hidden />
      <span className="flex-1">Picking up your unsaved edit</span>
      <button type="button" onClick={startOver} className={`${GLAZE_PILL} h-6 px-2.5 text-[12px]`}>
        Start over
      </button>
    </div>
  );

  const failLine = phase === 'failed' && entry.line ? `Couldn't save. ${entry.line} Your changes are still here.` : null;

  const footer = (
    <FooterRow layout={layout} hints={[['Enter', 'save'], ['Esc', 'close']]}>
      {check?.changed && phase !== 'done' && (
        <SecondaryButton layout={layout} onClick={startOver} disabled={phase === 'working'}>
          Discard
        </SecondaryButton>
      )}
      <MainButton layout={layout} phase={phase} label="Save" working="Saving" done="Saved" onClick={() => void save()} disabled={!canSave} />
    </FooterRow>
  );

  const picker = !sheet && pickerOpen && info && (
    <TitleEmojiPicker
      anchorRef={emojiButtonRef}
      recent={info.title_emoji}
      recentHeading={info.title_emoji_heading}
      onPick={insertEmoji}
      onClose={() => setPickerOpen(false)}
    />
  );

  // Twitch's stream info could not be read: never offer a form that would
  // save empty fields (no tags) over the channel's.
  if (!info) {
    const why = view.problems.find((p) => p.startsWith('Stream info')) ?? 'Stream info could not be read right now.';
    return (
      <FormShell layout={layout} frame={frameOf('info', { view, layout, onClose })} problem={why}>
        <p className="text-[13px] text-textSecondary">Open it again in a moment to try once more.</p>
      </FormShell>
    );
  }

  return (
    <FormShell layout={layout} frame={frameOf('info', { view, layout, onClose, onExpand })} footer={footer} problem={failLine}>
      <div onKeyDown={onEditorKey} className={big ? 'space-y-4' : 'space-y-3.5'}>
        {resumeLine}
        {big ? (
          <div className="grid grid-cols-1 gap-6 min-[760px]:grid-cols-[minmax(0,1fr)_260px]">
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
        ) : (
          <>
            {titleField}
            {categoryField}
            {underCategory(
              <>
                {tagsField}
                {languageRow}
                {previewCompact}
              </>,
            )}
          </>
        )}
      </div>
      {picker}
    </FormShell>
  );
}

// ─── raid ────────────────────────────────────────────────────────────────────

const RING = 52;
const RING_C = 2 * Math.PI * RING;

/** The raid's countdown as a ring that empties, with m:ss in its middle. */
function RaidRing({ deadline, total, now, instant }: { deadline: number; total: number; now: number; instant: boolean }) {
  const left = Math.max(0, deadline - now);
  const frac = total > 0 ? Math.min(1, left / total) : 0;
  return (
    <div className="relative mx-auto h-[128px] w-[128px]" role="timer" aria-label={`Raid in ${countdown(deadline, now)}`}>
      <svg viewBox="0 0 128 128" className="h-full w-full -rotate-90" aria-hidden>
        <circle cx="64" cy="64" r={RING} fill="none" strokeWidth="6" className="stroke-textPrimary/[0.08]" />
        <circle
          cx="64"
          cy="64"
          r={RING}
          fill="none"
          strokeWidth="6"
          strokeLinecap="round"
          stroke="var(--color-accent)"
          strokeDasharray={RING_C}
          strokeDashoffset={RING_C * (1 - frac)}
          style={{ transition: instant ? 'none' : 'stroke-dashoffset 250ms linear' }}
        />
      </svg>
      <span className="absolute inset-0 grid place-items-center text-[26px] font-bold tabular-nums text-textPrimary">
        {countdown(deadline, now)}
      </span>
    </div>
  );
}

const avatarOf = (url: string | null, px: number) =>
  url ? (
    <img src={url} alt="" className="flex-shrink-0 rounded-full" style={{ width: px, height: px }} />
  ) : (
    <span className="flex-shrink-0 rounded-full bg-textPrimary/10" style={{ width: px, height: px }} />
  );

/**
 * Raid: search, pick a live channel, then confirm once (Start raid / Back).
 * While the raid counts down it shows as a ring with Cancel and Raid now.
 */
function RaidForm({ view, layout, onClose, onExpand }: FormProps) {
  const v = useManagerStore();
  const ch = view.channel_id;
  const key = toolKey(ch, 'raid');
  const cancelKey = toolKey(ch, 'raid:cancel');
  const nowKey = toolKey(ch, 'raid:now');
  const entry = readTool(v, key);
  const d = (entry.draft as RaidDraft | undefined) ?? { query: '', target: null };
  const put = (next: RaidDraft) => setToolDraft(key, next);
  const [results, setResults] = useState<RaidTarget[]>([]);
  const [index, setIndex] = useState(0);
  const raid = view.raid;
  const now = useNow(raid ? 250 : 0);
  const instant = useInstant();
  const closeSoon = useCloseSoon(onClose);
  const sheet = layout === 'sheet';
  const confirmRef = useRef<HTMLDivElement>(null);
  const targetId = d.target?.id ?? null;

  // The confirm step takes the keys (Enter starts, Esc goes back) when it opens.
  useEffect(() => {
    if (targetId && !sheet) confirmRef.current?.focus();
  }, [targetId, sheet]);

  useEffect(() => {
    if (raid || d.target || !d.query.trim()) return;
    let alive = true;
    const t = window.setTimeout(() => {
      void searchRaidTargets(d.query, ch)
        .then((r) => {
          if (!alive) return;
          setResults(r);
          setIndex(Math.max(0, r.findIndex((x) => x.live)));
        })
        .catch(() => alive && setResults([]));
    }, 220);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [d.query, d.target, raid, ch]);

  const start = async (repeat: boolean) => {
    const action = raidStart(d.target, repeat);
    if (!action) return;
    await runTool(key, () => managerAct(ch, view.channel_login, action), { clearDraft: true });
  };

  const frame = frameOf('raid', { view, layout, onClose, onExpand });
  // The three steps share one height in the panel, so it never jumps
  // between search, confirm and the countdown.
  const step = layout === 'panel' ? 'min-h-[232px]' : '';

  if (raid) {
    const cancel = readTool(v, cancelKey);
    const go = readTool(v, nowKey);
    const footer = (
      <FooterRow layout={layout} hints={[['Esc', 'close']]}>
        <SecondaryButton
          layout={layout}
          disabled={cancel.phase === 'working' || go.phase === 'working'}
          onClick={() => void runTool(cancelKey, () => managerAct(ch, view.channel_login, { kind: 'cancel_raid' }))}
        >
          {cancel.phase === 'working' ? 'Canceling' : 'Cancel raid'}
        </SecondaryButton>
        <MainButton
          layout={layout}
          phase={go.phase}
          label="Raid now"
          working="Sending"
          done="Sent"
          disabled={cancel.phase === 'working'}
          onClick={async () => {
            const ok = await runTool(nowKey, () => managerAct(ch, view.channel_login, { kind: 'raid_now' }));
            if (ok && layout !== 'window') closeSoon();
          }}
        />
      </FooterRow>
    );
    return (
      <FormShell layout={layout} frame={frame} footer={footer} problem={failWords(cancel) ?? failWords(go)}>
        <div className={`flex flex-col items-center justify-center gap-3 pt-1 text-center ${step}`}>
          <RaidRing deadline={raid.deadline_ms} total={raid.total_ms} now={now} instant={instant} />
          <p className="text-[13.5px] text-textSecondary">
            Raiding <span className="font-semibold text-textPrimary">{raid.target_name}</span>
          </p>
        </div>
      </FormShell>
    );
  }

  if (d.target) {
    const t = d.target;
    const footer = (
      <FooterRow layout={layout} hints={[['Enter', 'start'], ['Esc', 'back']]}>
        <SecondaryButton layout={layout} onClick={() => put({ ...d, target: null })} disabled={entry.phase === 'working'}>
          <span className="flex items-center gap-1.5">
            <ArrowLeft size={14} aria-hidden />
            Back
          </span>
        </SecondaryButton>
        <MainButton layout={layout} phase={entry.phase} label="Start raid" working="Starting" done="Started" onClick={() => void start(false)} />
      </FooterRow>
    );
    return (
      <FormShell layout={layout} frame={frame} footer={footer} problem={failWords(entry)}>
        <div
          tabIndex={-1}
          ref={confirmRef}
          onKeyDown={(e) => {
            if (isImeComposing(e)) return;
            if (e.key === 'Enter') {
              e.preventDefault();
              void start(e.repeat);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              put({ ...d, target: null });
            }
          }}
          className={`flex flex-col items-center justify-center gap-2 pt-1 text-center outline-none ${step}`}
        >
          {avatarOf(t.avatar_url, 56)}
          <p className="text-[15px] font-semibold text-textPrimary">Raid {t.name}?</p>
          <p className="text-[12.5px] text-textSecondary">{t.line}</p>
          <p className="max-w-[300px] text-[12.5px] leading-snug text-textSecondary">
            Your viewers go with you when the countdown ends. You can cancel until then.
          </p>
        </div>
      </FormShell>
    );
  }

  const shown = d.query.trim() ? results : [];
  return (
    <FormShell layout={layout} frame={frame} footer={layout === 'sheet' ? undefined : <FooterRow layout={layout} hints={[['Enter', 'pick'], ['Esc', 'close']]}>{null}</FooterRow>}>
      <div className={`space-y-2 ${step}`}>
        <div className={`${FIELD} flex items-center gap-2`}>
          <Search size={15} className="flex-shrink-0 text-textSecondary" aria-hidden />
          <input
            autoFocus={!sheet}
            value={d.query}
            onChange={(e) => put({ query: e.target.value, target: null })}
            onKeyDown={(e) => {
              if (isImeComposing(e)) return;
              if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                e.preventDefault();
                const step = e.key === 'ArrowDown' ? 1 : -1;
                setIndex((i) => Math.max(0, Math.min(shown.length - 1, i + step)));
              } else if (e.key === 'Enter') {
                e.preventDefault();
                const pick = shown[index];
                if (pick?.live) put({ ...d, target: pick });
              }
            }}
            placeholder="Search for a channel"
            className="h-10 min-w-0 flex-1 bg-transparent text-sm text-textPrimary placeholder-textSecondary outline-none"
          />
        </div>
        {shown.map((t, i) => (
          <button
            key={t.id}
            type="button"
            disabled={!t.live}
            onClick={() => put({ ...d, target: t })}
            className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-textPrimary/[0.06] disabled:opacity-50 disabled:hover:bg-transparent ${
              i === index && t.live ? 'bg-textPrimary/[0.07]' : ''
            }`}
          >
            {avatarOf(t.avatar_url, 28)}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-semibold text-textPrimary">{t.name}</span>
              <span className="block truncate text-xs text-textSecondary">{t.line}</span>
            </span>
          </button>
        ))}
      </div>
    </FormShell>
  );
}

// ─── ads ─────────────────────────────────────────────────────────────────────

/** Ad break: the next ad counting down live, a length, Snooze and Run. */
function AdsForm({ view, layout, onClose, onExpand }: FormProps) {
  const v = useManagerStore();
  const ads = view.ads;
  const ch = view.channel_id;
  const runKey = toolKey(ch, 'ads:run');
  const snoozeKey = toolKey(ch, 'ads:snooze');
  const runEntry = readTool(v, runKey);
  const snooze = readTool(v, snoozeKey);
  const seconds = (runEntry.draft as number | undefined) ?? ads?.lengths[1]?.value ?? 60;
  const s = ads?.schedule ?? null;
  const next = s?.next_ad_ms ?? null;
  const now = useNow(next ? 1000 : 0);
  const closeSoon = useCloseSoon(onClose);
  if (!ads) return null;
  const detail = view.sections.find((x) => x.id === 'ads')?.detail;
  // Within the hour it counts down to the second; further off Rust's words.
  const soon = next !== null && next > now && next - now < 3_600_000;
  // The countdown ends its line, so nothing after it moves as it ticks; the
  // snoozes go on their own quieter line.
  const nextLine = soon ? `Next ad in ${countdown(next, now)}` : detail;
  const snoozeLine = soon ? (s?.snooze_line ?? null) : null;

  const footer = (
    <FooterRow layout={layout} hints={[['Esc', 'close']]}>
      {s && s.snoozes_left > 0 && (
        <SecondaryButton
          layout={layout}
          disabled={snooze.phase === 'working' || snooze.phase === 'done'}
          onClick={() => void runTool(snoozeKey, () => managerAct(ch, view.channel_login, { kind: 'snooze_ad' }))}
        >
          {snooze.phase === 'working' ? 'Snoozing' : snooze.phase === 'done' ? 'Snoozed' : 'Snooze next ad'}
        </SecondaryButton>
      )}
      <MainButton
        layout={layout}
        phase={runEntry.phase}
        label="Run ad break"
        working="Starting"
        done="Started"
        onClick={async () => {
          const ok = await runTool(runKey, () => managerAct(ch, view.channel_login, { kind: 'run_ad', seconds }));
          if (ok && layout !== 'window') closeSoon();
        }}
      />
    </FooterRow>
  );

  return (
    <FormShell layout={layout} frame={frameOf('ads', { view, layout, onClose, onExpand })} footer={footer} problem={failWords(runEntry) ?? failWords(snooze)}>
      {(nextLine || s?.preroll_line || ads.schedule_hint) && (
        <div className="space-y-0.5">
          {nextLine && <p className="text-[13.5px] text-textPrimary">{nextLine}</p>}
          {snoozeLine && <p className="text-xs text-textSecondary">{snoozeLine}</p>}
          {s?.preroll_line && <p className="text-xs text-textSecondary">{s.preroll_line}</p>}
          {ads.schedule_hint && <p className="text-xs text-textSecondary">{ads.schedule_hint}</p>}
        </div>
      )}
      <div>
        <p className={`${LABEL} mb-1.5`}>Length</p>
        <Segmented
          layout={layout}
          ariaLabel="Ad length"
          value={seconds}
          options={ads.lengths.map((l) => ({ value: l.value, label: l.label }))}
          onChange={(v) => setToolDraft(runKey, v)}
        />
      </div>
    </FormShell>
  );
}

// ─── rewards queue ───────────────────────────────────────────────────────────

/**
 * Rewards queue: who redeemed what, oldest first, by reward. A row you
 * complete or reject says Done or Refunded, then slides out.
 */
function RewardsQueue({ view, layout, queue, onQueue, onClose, onExpand }: FormProps & QueueProps) {
  const v = useManagerStore();
  const [error, setError] = useState<string | null>(null);
  const [leaving, setLeaving] = useState<LeavingRow[]>([]);
  const instant = useInstant();
  const ch = view.channel_id;
  const sheet = layout === 'sheet';

  useEffect(() => {
    let alive = true;
    void getRewardQueue(ch)
      .then((q) => {
        if (!alive) return;
        onQueue(ch, q);
        setError(null);
      })
      .catch((e) => alive && setError(typeof e === 'string' ? e : 'Could not read the queue'));
    return () => {
      alive = false;
    };
  }, [ch, onQueue]);

  const rowKey = (id: string) => toolKey(ch, `rewards:${id}`);
  const allKeyOf = (rewardId: string) => toolKey(ch, `rewards:all:${rewardId}`);
  const groups = withHeld(queue?.groups ?? [], leaving, (l) => isShowing(readTool(v, l.key)));

  /** Rows acted on stay until their write settles; settled ones are let go. */
  const hold = (rows: LeavingRow[]) =>
    setLeaving((l) => [...l.filter((x) => !rows.some((r) => r.row.id === x.row.id) && isShowing(getTool(x.key))), ...rows]);

  const actOn = (g: QueueGroup, r: QueueRow, index: number, fulfill: boolean) => {
    const key = rowKey(r.id);
    if (getTool(key).phase === 'working') return;
    hold([{ row: r, group: g, index, key }]);
    void runTool(key, () => managerAct(ch, view.channel_login, { kind: 'redemptions', ids: [r.id], fulfill }), {
      draft: { fulfill },
      forget: true,
    });
  };

  const completeAll = (g: QueueGroup) => {
    const key = allKeyOf(g.reward_id);
    if (getTool(key).phase === 'working') return;
    hold(g.rows.map((row, index) => ({ row, group: g, index, key })));
    void runTool(key, () => managerAct(ch, view.channel_login, { kind: 'reward_queue', reward_id: g.reward_id, fulfill: true }));
  };

  const body = error ? (
    <p className="py-2 text-sm text-textSecondary">{error}</p>
  ) : !queue ? (
    <p className="py-2 text-sm text-textSecondary">Loading…</p>
  ) : groups.length === 0 ? (
    <p className="py-2 text-sm text-textSecondary">{queue.empty_line ?? 'Nothing waiting'}</p>
  ) : (
    <div className="space-y-3">
      {groups.map((g) => {
        const pauseKey = toolKey(ch, `reward:${g.reward_id}`);
        const pause = readTool(v, pauseKey);
        const all = readTool(v, allKeyOf(g.reward_id));
        // Only refusals add a line; Complete all's result takes its button's place.
        const headLine = failWords(pause) ?? failWords(all);
        return (
          <div key={g.reward_id}>
            <div className="mb-1 flex min-h-7 items-center gap-2 px-2.5">
              <span className="min-w-0 flex-1 truncate text-xs font-semibold text-textSecondary">{g.heading}</span>
              <RowButton
                layout={layout}
                disabled={pause.phase === 'working'}
                onClick={() =>
                  void runTool(pauseKey, () => managerAct(ch, view.channel_login, { kind: 'reward', reward_id: g.reward_id, paused: !g.paused }))
                }
              >
                {g.paused ? 'Resume' : 'Pause'}
              </RowButton>
              {all.phase === 'working' ? (
                <LoaderCircle size={15} className="animate-spin text-textSecondary" aria-label="Completing all" />
              ) : all.phase === 'done' ? (
                <span role="status" className="flex items-center gap-1 text-[12px] font-semibold text-success">
                  <Check size={14} aria-hidden />
                  {all.line ?? 'Done'}
                </span>
              ) : (
                g.rows.length > 1 && (
                  <RowButton layout={layout} onClick={() => completeAll(g)}>
                    Complete all
                  </RowButton>
                )
              )}
            </div>
            {headLine && (
              <div className="mb-1">
                <ResultLine line={headLine} failed={pause.phase === 'failed' || all.phase === 'failed'} />
              </div>
            )}
            <div className="overflow-hidden rounded-lg bg-textPrimary/[0.04]">
              <AnimatePresence initial={false} mode="popLayout">
                {g.rows.map((r, i) => {
                  const e = readTool(v, rowKey(r.id));
                  const how = (e.draft as { fulfill: boolean } | undefined)?.fulfill;
                  return (
                    <motion.div
                      key={r.id}
                      layout={instant ? false : 'position'}
                      initial={false}
                      exit={{ opacity: 0, x: 24 }}
                      transition={easeFor(instant)}
                      className={`flex gap-2.5 border-t border-textPrimary/[0.05] px-2.5 py-2 first:border-t-0 ${r.input ? 'items-start' : 'items-center'}`}
                    >
                      {avatarOf(r.avatar_url, 24)}
                      <div className="min-w-0 flex-1">
                        {/* With nothing typed, who and when is the whole row: full size. */}
                        <p className={`truncate ${r.input ? 'text-[11px] text-textSecondary' : 'text-[13px] text-textPrimary'}`}>{r.line}</p>
                        {r.input && <p className="break-words text-sm text-textPrimary">{r.input}</p>}
                        {e.phase === 'failed' && <ResultLine line={e.line} failed />}
                      </div>
                      <div className="flex flex-shrink-0 items-center gap-1" role="status">
                        {e.phase === 'working' ? (
                          <LoaderCircle size={15} className="animate-spin text-textSecondary" aria-label="Working" />
                        ) : e.phase === 'done' ? (
                          <span className={`flex items-center gap-1 text-[12px] font-semibold ${how ? 'text-success' : 'text-textSecondary'}`}>
                            {how && <Check size={14} aria-hidden />}
                            {how ? 'Done' : 'Refunded'}
                          </span>
                        ) : isShowing(all) ? null : (
                          <>
                            <button
                              type="button"
                              onClick={() => actOn(g, r, i, true)}
                              className={`${ACCENT_BUTTON} ${sheet ? 'h-9 px-3.5 !text-[14px]' : 'h-7 px-3 !text-[12px]'}`}
                              style={ACCENT_FILL}
                            >
                              Complete
                            </button>
                            <Tooltip content="Refunds the points" side="top">
                              <button
                                type="button"
                                onClick={() => actOn(g, r, i, false)}
                                className={`${GLAZE_PILL} ${sheet ? 'h-9 px-3.5 text-[14px]' : 'h-7 px-3 text-[12px]'}`}
                              >
                                Reject
                              </button>
                            </Tooltip>
                          </>
                        )}
                      </div>
                    </motion.div>
                  );
                })}
              </AnimatePresence>
            </div>
          </div>
        );
      })}
      {queue.more && <p className="text-xs text-textSecondary">More are waiting; they show here as these clear.</p>}
    </div>
  );

  return (
    <FormShell
      layout={layout}
      frame={{ ...frameOf('rewards', { view, layout, onClose, onExpand }), badge: queue && queue.count > 0 ? <span className="text-[12px] tabular-nums text-textSecondary">{queue.count}</span> : undefined }}
    >
      {body}
    </FormShell>
  );
}

// ─── announce ────────────────────────────────────────────────────────────────

/** Twitch's announcement colors, for the live preview's edge. */
const ANNOUNCE_EDGE: Record<string, string> = {
  primary: 'var(--color-accent)',
  blue: 'linear-gradient(#00d6d6, #9146ff)',
  green: 'linear-gradient(#00db84, #57bee6)',
  orange: 'linear-gradient(#ffb31a, #e0e000)',
  purple: 'linear-gradient(#9146ff, #ff75e6)',
};
const ANNOUNCE_MAX = 500;

interface AnnounceDraft {
  text: string;
  color: string;
}

/** Announce: the text, its color, and how it will look in chat. */
function AnnounceForm({ view, layout, onClose, onExpand }: FormProps) {
  const ch = view.channel_id;
  const key = toolKey(ch, 'announce');
  const entry = useTool(key);
  const d = (entry.draft as AnnounceDraft | undefined) ?? { text: '', color: 'primary' };
  const colors = view.chat?.announce_colors ?? [];
  const count = codePoints(d.text.trim());
  const over = count > ANNOUNCE_MAX;
  const closeSoon = useCloseSoon(onClose);
  const sheet = layout === 'sheet';
  const ready = count > 0 && !over;

  const send = async () => {
    if (!ready) return;
    const ok = await runTool(key, () => managerAct(ch, view.channel_login, { kind: 'announce', text: d.text, color: d.color }), {
      clearDraft: true,
    });
    if (ok && layout !== 'window') closeSoon();
  };

  const footer = (
    <FooterRow layout={layout} hints={[['Enter', 'send'], ['Shift+Enter', 'new line']]}>
      <MainButton layout={layout} phase={entry.phase} label="Announce" working="Sending" done="Announced" onClick={() => void send()} disabled={!ready} />
    </FooterRow>
  );

  return (
    <FormShell layout={layout} frame={frameOf('announce', { view, layout, onClose, onExpand })} footer={footer} problem={failWords(entry)}>
      <div className={`glass-input !rounded-xl ${over ? '!border-error/60' : 'focus-within:!border-accent/40'}`}>
        <textarea
          autoFocus={!sheet}
          value={d.text}
          rows={3}
          aria-label="Your announcement"
          onChange={(e) => setToolDraft(key, { ...d, text: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !isImeComposing(e)) {
              e.preventDefault();
              void send();
            }
          }}
          placeholder="Your announcement"
          className="block w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-sm text-textPrimary placeholder-textSecondary outline-none"
        />
        <div className="flex justify-end px-3 pb-1.5">
          <span className={`text-[11.5px] font-semibold tabular-nums ${over ? 'text-error' : count > ANNOUNCE_MAX - 50 ? 'text-warning' : 'text-textSecondary'}`}>
            {count} / {ANNOUNCE_MAX}
          </span>
        </div>
      </div>
      {colors.length > 0 && (
        <div className="flex items-center gap-1.5">
          <span className={`${LABEL} mr-1`}>Color</span>
          {colors.map((c) => (
            <Tooltip key={c.id} content={c.label} side="top">
              <button
                type="button"
                onClick={() => setToolDraft(key, { ...d, color: c.id })}
                aria-label={c.label}
                aria-pressed={d.color === c.id}
                className={`grid h-7 w-7 place-items-center rounded-full transition-shadow ${d.color === c.id ? 'ring-2 ring-textPrimary/60' : ''}`}
              >
                <span className="h-4 w-4 rounded-full" style={{ background: ANNOUNCE_EDGE[c.id] ?? 'var(--color-accent)' }} />
              </button>
            </Tooltip>
          ))}
        </div>
      )}
      <div>
        <p className={`${LABEL} mb-1.5`}>In chat</p>
        <div className="flex overflow-hidden rounded-lg bg-surface/30 ring-1 ring-inset ring-textPrimary/[0.06]">
          <span className="w-[4px] flex-shrink-0" style={{ background: ANNOUNCE_EDGE[d.color] ?? 'var(--color-accent)' }} aria-hidden />
          <div className="min-w-0 flex-1 px-3 py-2">
            <p className="flex items-center gap-1.5 text-[11.5px] font-semibold text-textSecondary">
              <Megaphone size={13} aria-hidden />
              Announcement
            </p>
            <p className="mt-1 break-words text-[13.5px] text-textPrimary">
              <span className="font-semibold">{view.channel_login}</span>: {d.text.trim() || 'Your announcement'}
            </p>
          </div>
        </div>
      </div>
    </FormShell>
  );
}

// ─── running poll and prediction ─────────────────────────────────────────────

/**
 * A running poll or prediction: its name, then its controls (End keeps the
 * result on screen, Remove hides it; Lock, "{outcome} won" once locked,
 * Refund). The result shows on the row.
 */
export function LiveRow({ view, layout, which }: { view: StreamManagerView; layout: Layout | 'menu'; which: 'poll' | 'prediction' }) {
  const ch = view.channel_id;
  const key = toolKey(ch, `live:${which}`);
  const entry = useTool(key);
  const busy = entry.phase === 'working';
  const row = view.sections.find((s) => s.id === which);
  const poll = which === 'poll' ? view.poll : null;
  const prediction = which === 'prediction' ? view.prediction : null;
  const label = prediction?.status === 'locked' ? 'Prediction locked' : (row?.label ?? (poll ? 'Poll running' : 'Prediction running'));
  const title = poll?.title ?? prediction?.title ?? '';
  // What the row said when it was pressed: once the write ends the poll or
  // prediction, the row stays with its result until the result has shown.
  const pressed = entry.draft as { label: string; title: string } | undefined;
  const run = (action: Parameters<typeof managerAct>[2]) =>
    void runTool(key, () => managerAct(ch, view.channel_login, action), { draft: { label, title } });
  const settled = !poll && !prediction;
  if (settled && !(isShowing(entry) && pressed)) return null;
  const shownLabel = settled ? pressed!.label : label;
  const shownTitle = settled ? pressed!.title : title;
  const pad = layout === 'sheet' ? 'px-3 py-2.5' : layout === 'window' ? 'px-3.5 py-3' : 'px-3 py-2';
  if (settled) {
    return (
      <div className={pad}>
        <div className="flex items-center gap-2.5">
          <span className="flex w-4 flex-shrink-0 items-center justify-center text-textSecondary" aria-hidden>
            {which === 'poll' ? <BarChart3 size={14} /> : <Trophy size={14} />}
          </span>
          <span className="min-w-0 flex-1">
            <span className={`block truncate ${layout === 'sheet' ? 'text-[15px]' : 'text-[13.5px]'} text-textPrimary`}>{shownLabel}</span>
            {shownTitle && <span className="block truncate text-[12px] text-textSecondary">{shownTitle}</span>}
          </span>
          {busy ? (
            <LoaderCircle size={14} className="animate-spin text-textSecondary" aria-label="Working" />
          ) : (
            <span role="status" className="flex flex-shrink-0 items-center gap-1 text-[12px] font-semibold text-success">
              <Check size={14} aria-hidden />
              {entry.line}
            </span>
          )}
        </div>
      </div>
    );
  }
  return (
    <div className={pad}>
      <div className="flex items-center gap-2.5">
        <span className="flex w-4 flex-shrink-0 items-center justify-center text-accent" aria-hidden>
          {poll ? <BarChart3 size={14} /> : <Trophy size={14} />}
        </span>
        <span className="min-w-0 flex-1">
          <span className={`block truncate ${layout === 'sheet' ? 'text-[15px]' : 'text-[13.5px]'} text-textPrimary`}>{label}</span>
          {title && <span className="block truncate text-[12px] text-textSecondary">{title}</span>}
        </span>
        {busy && <LoaderCircle size={14} className="animate-spin text-textSecondary" aria-label="Working" />}
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5 pl-[26px]">
        {poll && (
          <>
            <RowButton layout={layout} disabled={busy} onClick={() => run({ kind: 'end_poll', show_results: true })}>
              End
            </RowButton>
            <RowButton layout={layout} disabled={busy} onClick={() => run({ kind: 'end_poll', show_results: false })}>
              Remove
            </RowButton>
          </>
        )}
        {prediction?.status === 'active' && (
          <RowButton layout={layout} disabled={busy} onClick={() => run({ kind: 'lock_prediction' })}>
            Lock
          </RowButton>
        )}
        {prediction?.status === 'locked' &&
          prediction.outcomes.map((o) => (
            <RowButton key={o.id} layout={layout} disabled={busy} onClick={() => run({ kind: 'resolve_prediction', outcome_id: o.id })}>
              {o.title} won
            </RowButton>
          ))}
        {prediction && (
          <RowButton layout={layout} tone="danger" disabled={busy} onClick={() => run({ kind: 'cancel_prediction' })}>
            Refund
          </RowButton>
        )}
      </div>
      {(entry.phase === 'done' || entry.phase === 'failed') && (
        <div className="mt-1.5 pl-[26px]">
          <ResultLine line={entry.line} failed={entry.phase === 'failed'} />
        </div>
      )}
    </div>
  );
}

/** The window's page for a running poll or prediction. */
export function LivePage({ view, which }: { view: StreamManagerView; which: 'poll' | 'prediction' }) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto scrollbar-thin px-5 pb-5 pt-1">
      <GroupCard title={which === 'poll' ? 'Running now' : 'Open now'} layout="window" hideTitle>
        <LiveRow view={view} layout="window" which={which} />
      </GroupCard>
    </div>
  );
}

export default StreamManagerPanel;
