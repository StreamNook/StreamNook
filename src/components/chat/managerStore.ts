// What the stream manager's forms hold for this run of the app: each tool's
// draft and the state of its last write, keyed by channel and tool, so a
// result is still there after the panel closes, opens larger, collapses back
// or reopens, and a write for one channel never shows on another.
//
// This is window UI state only. Rust decides who may do what, words every
// line and runs every write (services/stream_manager.rs); nothing here is
// shared between windows or stored.
import { useSyncExternalStore } from 'react';
import type { ManagerAction, PageId, PageRow, RaidTarget, QueueGroup, QueueRow } from '../../services/streamManager';

export type ToolPhase = 'idle' | 'working' | 'done' | 'failed';

export interface ToolEntry {
  phase: ToolPhase;
  /** Rust's line for the last write: what happened, or why not. */
  line: string | null;
  /** Counts writes on this key, so a timer from an older write never ends a newer one. */
  gen: number;
  /** What the form holds (typed by the form that owns the key). */
  draft: unknown;
  /** For Stream info: what Twitch had when the edit began. */
  base: string | null;
  /** The draft a write in flight was sent with. */
  sent: unknown;
}

export const IDLE: ToolEntry = Object.freeze({ phase: 'idle', line: null, gen: 0, draft: undefined, base: null, sent: undefined }) as ToolEntry;

/** How long "Done" shows before a key goes back to idle. */
export const DONE_HOLD_MS = 1400;
/** How long a closing panel shows its result before it closes itself. */
export const SAVED_HOLD_MS = 900;

const entries = new Map<string, ToolEntry>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();
const listeners = new Set<() => void>();
let version = 0;

function notify(): void {
  version += 1;
  for (const l of [...listeners]) l();
}

function write(key: string, next: ToolEntry): void {
  entries.set(key, next);
  notify();
}

/** The key for one tool of one channel. */
export const toolKey = (channelId: string, tool: string): string => `${channelId}:${tool}`;

/** A key's entry now. For event handlers and timers; a render reads through
 *  `useTool` or `readTool`, so the React Compiler never keeps an old answer. */
export function getTool(key: string): ToolEntry {
  return entries.get(key) ?? IDLE;
}

/** A write is running or its result still shows: whatever it acted on stays
 *  on screen (a row the write removed, a running poll it ended). */
export const isShowing = (e: ToolEntry): boolean => e.phase === 'working' || e.phase === 'done';

/** A running poll's or prediction's row is still showing its last write
 *  (End, Remove, a winner, Refund), though the view no longer has it. */
export const liveHeld = (storeVersion: number, channelId: string, which: 'poll' | 'prediction'): boolean =>
  isShowing(readTool(storeVersion, toolKey(channelId, `live:${which}`)));

/** A key's entry as of `storeVersion` (from `useManagerStore`), for a render
 *  that reads several keys. Passing the version is what makes a compiled
 *  render read again after the store changes. */
export function readTool(storeVersion: number, key: string): ToolEntry {
  void storeVersion;
  return entries.get(key) ?? IDLE;
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
const snapshot = () => version;

/** The store's version, changing on every write; pass it to `readTool`. */
export function useManagerStore(): number {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** One key's entry, re-rendering when it changes. */
export function useTool(key: string): ToolEntry {
  const read = () => entries.get(key) ?? IDLE;
  return useSyncExternalStore(subscribe, read, read);
}

/** Replaces the key's draft. A failed key goes back to idle: the edit is the
 *  answer to the failure. A working or done key keeps its phase. */
export function setToolDraft(key: string, draft: unknown, base?: string | null): void {
  const cur = getTool(key);
  const failed = cur.phase === 'failed';
  write(key, {
    ...cur,
    draft,
    base: base === undefined ? cur.base : base,
    phase: failed ? 'idle' : cur.phase,
    line: failed ? null : cur.line,
  });
}

/** Back to nothing typed (Discard, Start over, a finished write). */
export function clearToolDraft(key: string): void {
  const cur = getTool(key);
  if (cur.draft === undefined && cur.base === null) return;
  write(key, { ...cur, draft: undefined, base: null });
}

export interface RunAnswer {
  ok: boolean;
  message: string;
}

export interface RunOptions {
  /** Clear the draft after a success, if it is still the one that was sent. */
  clearDraft?: boolean;
  /** How long `done` shows before idle (default DONE_HOLD_MS). */
  holdMs?: number;
  /** Stored as the draft while the write runs (a chat mode's wanted state). */
  draft?: unknown;
  /** Drop the key once its done line has shown (one key per redemption). */
  forget?: boolean;
}

const ERR_NO_ANSWER = 'Twitch did not answer.';

/**
 * Runs one write on `key`. Refused (false) while that key is working, so a
 * second press never sends twice. The answer's line stays on the key: `done`
 * goes back to idle on its own after the hold, whether or not a form is still
 * mounted; `failed` stays until the next run or an edit. Resolves to whether
 * the write went through.
 */
export async function runTool(key: string, run: () => Promise<RunAnswer>, opts: RunOptions = {}): Promise<boolean> {
  const cur = getTool(key);
  if (cur.phase === 'working') return false;
  const old = timers.get(key);
  if (old) clearTimeout(old);
  timers.delete(key);
  const gen = cur.gen + 1;
  const draft = opts.draft === undefined ? cur.draft : opts.draft;
  write(key, { ...cur, draft, phase: 'working', line: null, gen, sent: draft });
  let answer: RunAnswer;
  try {
    answer = await run();
  } catch (e) {
    answer = { ok: false, message: typeof e === 'string' && e ? e : ERR_NO_ANSWER };
  }
  const now = getTool(key);
  if (now.gen !== gen) return answer.ok;
  if (!answer.ok) {
    write(key, { ...now, phase: 'failed', line: answer.message || ERR_NO_ANSWER, sent: undefined });
    return false;
  }
  const unchanged = now.draft === now.sent;
  const keepDraft = !(opts.clearDraft && unchanged);
  write(key, {
    ...now,
    phase: 'done',
    line: answer.message,
    sent: undefined,
    draft: keepDraft ? now.draft : undefined,
    base: keepDraft ? now.base : null,
  });
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      const e = getTool(key);
      if (e.gen !== gen || e.phase !== 'done') return;
      if (opts.forget) {
        entries.delete(key);
        notify();
      } else {
        write(key, { ...e, phase: 'idle', line: null });
      }
    }, opts.holdMs ?? DONE_HOLD_MS),
  );
  return true;
}

/** Tests only: forget everything. */
export function resetManagerStore(): void {
  for (const t of timers.values()) clearTimeout(t);
  timers.clear();
  entries.clear();
  notify();
}

// ─── which surface is open ────────────────────────────────────────────────────

/**
 * Whether a ChatWidget must close its stream manager panel, builder and
 * window now: the channel changed under them, or the manager no longer
 * applies here. Either way nothing open may write on.
 */
export function mustCloseSurfaces(was: { channelId: string | null | undefined }, now: { channelId: string | null | undefined; manages: boolean }, anyOpen: boolean): boolean {
  if (was.channelId !== now.channelId) return true;
  return anyOpen && !now.manages;
}

// ─── pages ────────────────────────────────────────────────────────────────────

const lastPages = new Map<string, PageId>();

export function rememberPage(channelId: string, page: PageId): void {
  lastPages.set(channelId, page);
}

export function lastPage(channelId: string): PageId | null {
  return lastPages.get(channelId) ?? null;
}

/** The tools that have a panel over the message box. */
export type PanelKind = 'info' | 'raid' | 'ads' | 'rewards' | 'announce';
const PANEL_KINDS: readonly PageId[] = ['info', 'raid', 'ads', 'rewards', 'announce'];
export const isPanelKind = (p: PageId): p is PanelKind => PANEL_KINDS.includes(p);

/** Where the window's Collapse goes from a page: its panel, the builder, or
 *  nowhere (Chat modes and Moderation live in the Tools menu; a running poll
 *  or prediction is its row there). */
export function collapseTarget(
  page: PageId,
  running: { poll: unknown; prediction: unknown },
): PanelKind | 'poll' | 'prediction' | null {
  if (isPanelKind(page)) return page;
  if (page === 'poll') return running.poll ? null : 'poll';
  if (page === 'prediction') return running.prediction ? null : 'prediction';
  return null;
}

/** The side list's rows under their headings, in Rust's order. */
export function groupPages(pages: PageRow[]): { group: string; rows: PageRow[] }[] {
  const out: { group: string; rows: PageRow[] }[] = [];
  for (const p of pages) {
    const last = out[out.length - 1];
    if (last && last.group === p.group) last.rows.push(p);
    else out.push({ group: p.group, rows: [p] });
  }
  return out;
}

/** The page the window opens on: the one asked for, else the last one used
 *  here, else the first; never one the role no longer has. */
export function initialPage(requested: PageId | null, last: PageId | null, pages: PageRow[]): PageId | null {
  const has = (p: PageId | null): p is PageId => !!p && pages.some((r) => r.id === p);
  if (has(requested)) return requested;
  if (has(last)) return last;
  return pages[0]?.id ?? null;
}

// ─── counting ─────────────────────────────────────────────────────────────────

/** Characters as Twitch's limits count them: code points, so an emoji is one,
 *  as Rust counts them (`chars()`). */
export function codePoints(s: string): number {
  return Array.from(s).length;
}

/** `s` cut to at most `max` code points. */
export function clampPoints(s: string, max: number): string {
  if (codePoints(s) <= max) return s;
  return Array.from(s).slice(0, max).join('');
}

// ─── the builder ──────────────────────────────────────────────────────────────

export type BuilderKind = 'poll' | 'prediction';

export interface BuilderChoice {
  id: string;
  text: string;
}

export interface BuilderDraft {
  kind: BuilderKind;
  title: string;
  choices: BuilderChoice[];
  seconds: number;
  points: number;
}

let choiceSeq = 0;
export const newChoiceId = (): string => `c${(choiceSeq += 1)}`;

export function freshBuilder(kind: BuilderKind): BuilderDraft {
  return { kind, title: '', choices: [{ id: newChoiceId(), text: '' }, { id: newChoiceId(), text: '' }], seconds: 120, points: 0 };
}

/** A new empty box once the last one has text, up to `max` boxes. */
export function growChoices(choices: BuilderChoice[], max: number): BuilderChoice[] {
  const last = choices[choices.length - 1];
  if (!last || !last.text.trim() || choices.length >= max) return choices;
  return [...choices, { id: newChoiceId(), text: '' }];
}

/** The same draft as the other kind: choices trimmed to its maximum (empty
 *  ones first), the length moved into its range, at least `min` boxes. */
export function switchKind(
  d: BuilderDraft,
  kind: BuilderKind,
  limits: { max: number; min: number; minSecs: number; maxSecs: number },
): BuilderDraft {
  if (d.kind === kind) return d;
  let choices = d.choices;
  while (choices.length > limits.max) {
    const empty = choices.findIndex((c) => !c.text.trim());
    choices = empty >= 0 ? choices.filter((_, i) => i !== empty) : choices.slice(0, limits.max);
  }
  // Padding ids come from the draft, so the same switch shown again in a later
  // render keeps the same boxes (and the caret in them).
  const taken = new Set(choices.map((c) => c.id));
  for (let n = 0; choices.length < limits.min; n += 1) {
    const id = `${kind}-pad-${n}`;
    if (!taken.has(id)) choices = [...choices, { id, text: '' }];
  }
  return { ...d, kind, choices, seconds: Math.min(Math.max(d.seconds, limits.minSecs), limits.maxSecs) };
}

/** Moves the choice at `i` by `delta` (Alt+Up/Down). */
export function moveChoice(choices: BuilderChoice[], i: number, delta: number): BuilderChoice[] {
  const j = i + delta;
  if (i < 0 || j < 0 || i >= choices.length || j >= choices.length) return choices;
  const next = [...choices];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

// ─── raid ─────────────────────────────────────────────────────────────────────

export interface RaidDraft {
  query: string;
  target: RaidTarget | null;
}

/** Start raid: the action to send, or nothing for a held key's repeat or
 *  with no channel picked. Picking a channel only fills the draft. */
export function raidStart(target: RaidTarget | null, repeat: boolean): ManagerAction | null {
  if (repeat || !target || !target.live) return null;
  return { kind: 'raid', target_id: target.id, target_login: target.login, target_name: target.name };
}

// ─── chat modes ───────────────────────────────────────────────────────────────

export type ChatModeKind = 'followers' | 'slow' | 'subs' | 'emotes' | 'unique';

/** One mode as the room has it now. */
export interface ModeState {
  on: boolean;
  /** Followers minutes or slow seconds while on. */
  value: number | null;
}

export type LiveModes = Record<ChatModeKind, ModeState>;

/** The desktop's live modes, from the room's IRC ROOMSTATE. */
export function liveFromRoom(rs: { followersOnly: number; slow: number; subsOnly: boolean; emoteOnly: boolean; r9k: boolean }): LiveModes {
  return {
    followers: { on: rs.followersOnly >= 0, value: rs.followersOnly >= 0 ? rs.followersOnly : null },
    slow: { on: rs.slow > 0, value: rs.slow > 0 ? rs.slow : null },
    subs: { on: rs.subsOnly, value: null },
    emotes: { on: rs.emoteOnly, value: null },
    unique: { on: rs.r9k, value: null },
  };
}

/** The phone's live modes, from Rust's view (read again after every write). */
export function liveFromView(modes: { kind: ChatModeKind; on: boolean; value: number | null }[]): LiveModes {
  const out = liveFromRoom({ followersOnly: -1, slow: 0, subsOnly: false, emoteOnly: false, r9k: false });
  for (const m of modes) out[m.kind] = { on: m.on, value: m.on ? m.value : null };
  return out;
}

/** The write for a press: a switch (`value` undefined keeps what is on, or
 *  Rust's default when turning on) or a length picked while on. */
export function chatModeAction(mode: ChatModeKind, on: boolean, value?: number | null): ManagerAction {
  return { kind: 'chat_mode', mode, on, value: on ? (value ?? null) : null };
}

/** True once the room shows what was asked for. A switch turned on without a
 *  length matches any length. */
export function modeMatches(wanted: ModeState, live: ModeState): boolean {
  if (wanted.on !== live.on) return false;
  return !wanted.on || wanted.value == null || wanted.value === live.value;
}

const sameMode = (a: ModeState, b: ModeState) => a.on === b.on && (a.value ?? null) === (b.value ?? null);

/** What was asked for on a mode, and the room as it was at the press. */
export interface ModeWanted extends ModeState {
  roomAtPress: ModeState;
}

/**
 * What a mode row shows. While the write runs: what was asked for. A refusal:
 * the room. After a write that went through, until the room's own line
 * (IRC ROOMSTATE) changes from what it was at the press: Rust's read after
 * the write (`afterWrite`, from its view), else what was asked for. Once the
 * room moves, the room.
 */
export function shownMode(entry: ToolEntry, live: ModeState, afterWrite: ModeState | null): ModeState {
  const wanted = entry.draft as ModeWanted | undefined;
  if (!wanted) return live;
  if (entry.phase === 'working') return wanted;
  if (entry.phase === 'failed') return live;
  if (modeMatches(wanted, live) || !sameMode(live, wanted.roomAtPress)) return live;
  return afterWrite ?? wanted;
}

// ─── rewards ──────────────────────────────────────────────────────────────────

/** A redemption acted on: shown with its result until its key goes idle,
 *  even after the queue Twitch sends no longer has it. */
export interface LeavingRow {
  row: QueueRow;
  /** Its reward as it was, so a reward whose last row left still shows. */
  group: QueueGroup;
  /** Where it sat in its reward's list. */
  index: number;
  /** The store key whose write holds it (its own, or its reward's Complete all). */
  key: string;
}

/** The queue with the rows acted on still in place (while `held`). A reward
 *  the queue no longer has comes back after the others, holding only those rows. */
export function withHeld(groups: QueueGroup[], leaving: LeavingRow[], held: (l: LeavingRow) => boolean): QueueGroup[] {
  const back = leaving.filter(held);
  if (!back.length) return groups;
  const put = (rows: QueueRow[], mine: LeavingRow[]) => {
    const out = [...rows];
    for (const l of [...mine].sort((a, b) => a.index - b.index)) {
      if (!out.some((r) => r.id === l.row.id)) out.splice(Math.min(l.index, out.length), 0, l.row);
    }
    return out;
  };
  const out = groups.map((g) => {
    const mine = back.filter((l) => l.group.reward_id === g.reward_id);
    return mine.length ? { ...g, rows: put(g.rows, mine) } : g;
  });
  for (const l of back) {
    if (out.some((g) => g.reward_id === l.group.reward_id)) continue;
    const mine = back.filter((x) => x.group.reward_id === l.group.reward_id);
    out.push({ ...l.group, rows: put([], mine) });
  }
  return out;
}
