import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Megaphone, Pencil, Send, Tv, X } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Tooltip } from '../ui/Tooltip';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { isImeComposing } from '../../utils/imeComposing';
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

const LABEL = 'text-[11px] font-semibold uppercase tracking-[0.14em] text-textSecondary';

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
      {kind === 'info' && <InfoForm view={view} busy={busy} act={act} />}
      {kind === 'raid' && <RaidForm view={view} busy={busy} act={act} />}
      {kind === 'ads' && <AdsForm view={view} busy={busy} act={act} />}
      {kind === 'rewards' && <RewardsQueue view={view} queue={queue} onQueue={onQueue} busy={busy} act={act} />}
      {kind === 'announce' && <AnnounceForm view={view} busy={busy} act={act} />}
    </>
  );
}

export function StreamManagerPanel({ kind, view, queue, onQueue, onOutcome, onClose }: Props) {
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
      className="sn-popover absolute bottom-full left-0 right-0 mb-2 z-50 flex flex-col max-h-[70vh] overflow-hidden"
    >
      <div className="flex items-center gap-2 px-3 py-2.5">
        <span className="flex items-center justify-center text-textSecondary">{head.icon}</span>
        <span className="flex-1 text-sm font-semibold text-textPrimary">{head.title}</span>
        <Tooltip content="Close" side="top">
          <button onClick={onClose} className="p-1 text-textSecondary hover:text-textPrimary transition-colors" aria-label="Close">
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

function InfoForm({ view, busy, act }: FormProps) {
  const info = view.info;
  const [draft, setDraft] = useState<StreamInfoDraft>(() => ({
    title: info?.title ?? '',
    category: info?.category ?? null,
    tags: info?.tags ?? [],
    language: info?.language ?? '',
  }));
  const [check, setCheck] = useState<InfoCheck | null>(null);
  const [tagText, setTagText] = useState('');
  const [catQuery, setCatQuery] = useState<string | null>(null);
  const [catResults, setCatResults] = useState<Category[]>([]);
  const titleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

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
      void invoke<Category[]>('search_categories', { query: catQuery.trim(), limit: 8 })
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
  const boxArt = (c: Category) => c.box_art_url.replace('{width}', '52').replace('{height}', '72');

  return (
    <div className="space-y-3">
      <div>
        <div className="flex items-baseline justify-between mb-1">
          <label className={LABEL}>Title</label>
          <span className="text-[10px] tabular-nums text-textSecondary">{check?.title_count ?? ''}</span>
        </div>
        <input
          ref={titleRef}
          value={draft.title}
          onChange={(e) => setDraft((d) => ({ ...d, title: e.target.value }))}
          className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary"
          placeholder="What are you streaming?"
        />
        {problemFor('title') && <p className="mt-1 text-[11px] text-error">{problemFor('title')}</p>}
      </div>

      <div className="relative">
        <label className={`${LABEL} block mb-1`}>Category</label>
        {catQuery === null ? (
          <button
            onClick={() => setCatQuery('')}
            className="glass-input w-full px-3 py-2 flex items-center gap-2.5 text-left text-sm text-textPrimary"
          >
            {draft.category?.box_art_url && (
              <img src={boxArt(draft.category)} alt="" className="w-[18px] h-[24px] rounded-sm object-cover flex-shrink-0" />
            )}
            <span className={draft.category ? '' : 'text-textSecondary'}>{draft.category?.name ?? 'Pick a category'}</span>
          </button>
        ) : (
          <>
            <input
              autoFocus
              value={catQuery}
              onChange={(e) => setCatQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== 'Escape') return;
                e.stopPropagation();
                setCatQuery(null);
              }}
              placeholder="Search categories"
              className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary"
            />
            {catQuery.trim() && catResults.length > 0 && (
              <div className="mt-1 rounded-md overflow-hidden hairline-y max-h-56 overflow-y-auto scrollbar-thin">
                {catResults.map((c) => (
                  <button
                    key={c.id}
                    onClick={() => {
                      setDraft((d) => ({ ...d, category: c }));
                      setCatQuery(null);
                    }}
                    className="w-full flex items-center gap-2.5 px-2.5 py-1.5 text-left hover:bg-surface-hover transition-colors"
                  >
                    <img src={boxArt(c)} alt="" className="w-[18px] h-[24px] rounded-sm object-cover flex-shrink-0" />
                    <span className="text-sm text-textPrimary truncate">{c.name}</span>
                  </button>
                ))}
              </div>
            )}
          </>
        )}
      </div>

      <div>
        <div className="flex items-baseline justify-between mb-1">
          <label className={LABEL}>Tags</label>
          <span className="text-[10px] tabular-nums text-textSecondary">{check?.tags_count ?? ''}</span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {draft.tags.map((t, i) => (
            <span
              key={`${t}-${i}`}
              className={`h-6 pl-2 pr-1 rounded-md flex items-center gap-1 text-xs ${
                problemFor(`tag:${i}`) ? 'bg-error/15 text-error' : 'bg-white/10 text-textSecondary'
              }`}
            >
              {t}
              <button
                aria-label={`Remove ${t}`}
                onClick={() => setDraft((d) => ({ ...d, tags: d.tags.filter((_, j) => j !== i) }))}
                className="p-0.5 hover:text-textPrimary"
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
                if (e.key === 'Enter' || e.key === ',') {
                  e.preventDefault();
                  addTag();
                } else if (e.key === 'Backspace' && !tagText && draft.tags.length) {
                  setDraft((d) => ({ ...d, tags: d.tags.slice(0, -1) }));
                }
              }}
              onBlur={addTag}
              placeholder="Add a tag"
              className="h-6 w-24 flex-1 min-w-[6rem] bg-transparent text-xs text-textPrimary placeholder-textSecondary outline-none"
            />
          )}
        </div>
        {tagProblem && <p className="mt-1 text-[11px] text-error">{tagProblem}</p>}
      </div>

      <div>
        <label className={`${LABEL} block mb-1`}>Language</label>
        <select
          value={draft.language}
          onChange={(e) => setDraft((d) => ({ ...d, language: e.target.value }))}
          className="glass-input w-full px-3 py-2 text-sm text-textPrimary"
        >
          {(info?.languages ?? []).map((l) => (
            <option key={l.code} value={l.code}>
              {l.name}
            </option>
          ))}
        </select>
      </div>

      {preview && (
        <div>
          <label className={`${LABEL} block mb-1`}>What viewers see</label>
          <div className="flex gap-2.5 p-2 rounded-lg bg-white/5">
            <div className="w-[112px] aspect-video rounded-md overflow-hidden bg-white/5 flex-shrink-0">
              {preview.thumbnail_url && <img src={preview.thumbnail_url} alt="" className="w-full h-full object-cover" />}
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-[13px] font-semibold text-textPrimary truncate">{preview.title || 'Your title'}</p>
              <p className="text-xs text-textSecondary truncate">{preview.channel_name}</p>
              <p className="text-xs text-textSecondary truncate">{preview.category_name}</p>
              {preview.tags.length > 0 && (
                <div className="mt-1 flex flex-wrap gap-1">
                  {preview.tags.slice(0, 4).map((t) => (
                    <span key={t} className="h-[18px] px-1.5 rounded bg-white/10 text-[10.5px] text-textSecondary flex items-center">
                      {t}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      <button
        onClick={() => void act({ kind: 'save_info', draft }, true)}
        disabled={busy || !check?.ok || !check.changed}
        className="glass-button w-full py-2 text-sm font-semibold text-textPrimary disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {busy ? 'Saving…' : check && !check.changed ? 'No changes' : 'Save'}
      </button>
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
            className="flex-1 py-2 text-sm font-semibold rounded-md bg-red-500/15 hover:bg-red-500/25 text-red-300 transition-colors disabled:opacity-50"
          >
            Cancel raid
          </button>
          <button
            onClick={() => void act({ kind: 'raid_now' }, true)}
            disabled={busy}
            className="glass-button flex-1 py-2 text-sm font-semibold text-textPrimary disabled:opacity-50"
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
          className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-md text-left hover:bg-white/5 transition-colors disabled:opacity-50 disabled:hover:bg-transparent"
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
              className={`flex-1 py-1.5 text-xs font-semibold rounded-md transition-colors ${
                seconds === l.value ? 'glass-button-active text-textPrimary' : 'text-textSecondary hover:text-textPrimary'
              }`}
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
            className="glass-button-secondary flex-1 py-2 text-sm font-semibold text-textPrimary disabled:opacity-50"
          >
            Snooze next ad
          </button>
        )}
        <button
          onClick={() => void act({ kind: 'run_ad', seconds }, true)}
          disabled={busy}
          className="glass-button flex-1 py-2 text-sm font-semibold text-textPrimary disabled:opacity-50"
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
              className="sm-btn sm-head px-2 py-0.5 text-[11px] font-semibold rounded bg-white/10 hover:bg-white/20 text-textPrimary transition-colors disabled:opacity-50"
            >
              {g.paused ? 'Resume' : 'Pause'}
            </button>
            {g.rows.length > 1 && (
              <button
                onClick={() => void act({ kind: 'reward_queue', reward_id: g.reward_id, fulfill: true })}
                disabled={busy}
                className="sm-btn sm-head px-2 py-0.5 text-[11px] font-semibold rounded bg-white/10 hover:bg-white/20 text-textPrimary transition-colors disabled:opacity-50"
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
                    className="sm-btn px-2 py-1 text-[11px] font-semibold rounded bg-success/15 hover:bg-success/25 text-success transition-colors disabled:opacity-50"
                  >
                    Complete
                  </button>
                  <Tooltip content="Refunds the points" side="top">
                    <button
                      onClick={() => void act({ kind: 'redemptions', ids: [r.id], fulfill: false })}
                      disabled={busy}
                      className="sm-btn px-2 py-1 text-[11px] font-semibold rounded bg-red-500/15 hover:bg-red-500/25 text-red-300 transition-colors disabled:opacity-50"
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
          className="glass-button ml-auto px-4 py-1.5 text-sm font-semibold text-textPrimary disabled:opacity-50"
        >
          Announce
        </button>
      </div>
    </div>
  );
}

export default StreamManagerPanel;
