// What the stream manager's forms hold for this run of the app: each tool's
// draft and the state of its last write, keyed by channel and tool, so a
// result is still there after the panel closes, opens larger, collapses back
// or reopens, and a write for one channel never shows on another.
//
// This is window UI state only. Rust decides who may do what, words every
// line and runs every write (services/stream_manager.rs); nothing here is
// shared between windows or stored.
import { useSyncExternalStore } from 'react';
import type {
  InfoField,
  ManagerAction,
  PageId,
  PageRow,
  RaidTarget,
  QueueGroup,
  QueueRow,
  StreamInfoDraft,
  StreamInfoView,
} from '../../services/streamManager';

export type ToolPhase = 'idle' | 'working' | 'done' | 'failed';

export interface ToolEntry {
  phase: ToolPhase;
  /** Rust's line for the last write: what happened, or why not. */
  line: string | null;
  /** This write's number, from one counter for the whole window that never
   *  goes back, so a timer from an older write (even one from before the
   *  store was emptied) never ends a newer one. */
  gen: number;
  /** What the form holds (typed by the form that owns the key). */
  draft: unknown;
  /** The draft a write in flight was sent with. */
  sent: unknown;
}

export const IDLE: ToolEntry = Object.freeze({ phase: 'idle', line: null, gen: 0, draft: undefined, sent: undefined }) as ToolEntry;

/** How long "Done" shows before a key goes back to idle. */
export const DONE_HOLD_MS = 1400;
/** How long a closing panel shows its result before it closes itself. */
export const SAVED_HOLD_MS = 900;

const entries = new Map<string, ToolEntry>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();
const listeners = new Set<() => void>();
let version = 0;
/** Every write's and every save's number. Never reset. */
let counter = 0;

/** The next number from the window's one counter. */
export function nextStamp(): number {
  counter += 1;
  return counter;
}

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
export function setToolDraft(key: string, draft: unknown): void {
  const cur = getTool(key);
  const failed = cur.phase === 'failed';
  write(key, {
    ...cur,
    draft,
    phase: failed ? 'idle' : cur.phase,
    line: failed ? null : cur.line,
  });
}

/** Back to nothing typed (Discard, Start over, a finished write). */
export function clearToolDraft(key: string): void {
  const cur = getTool(key);
  if (cur.draft === undefined) return;
  write(key, { ...cur, draft: undefined });
}

/** Replaces only the key's draft: its phase and line stay, so a failure's
 *  line is never wiped by something that is not an edit. */
export function replaceToolDraft(key: string, draft: unknown): void {
  const cur = getTool(key);
  if (cur.draft === draft) return;
  write(key, { ...cur, draft });
}

/** In `ms`, changes the key's draft with `change` (which gets the draft as it
 *  is then). Runs whether or not a form is mounted; `timerKey` names it, so a
 *  later call with the same name replaces it. */
export function changeDraftLater(key: string, timerKey: string, ms: number, change: (draft: unknown) => unknown): void {
  const old = timers.get(timerKey);
  if (old) clearTimeout(old);
  timers.set(
    timerKey,
    setTimeout(() => {
      timers.delete(timerKey);
      const cur = getTool(key);
      if (cur.draft === undefined) return;
      replaceToolDraft(key, change(cur.draft));
    }, ms),
  );
}

/** Counts `forgetAccount` calls, so work that began as one account can tell
 *  the store has been emptied for another since. */
let epoch = 0;

/** The store's account epoch now (`forgetAccount` moves it on). */
export function storeEpoch(): number {
  return epoch;
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
  const gen = nextStamp();
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

/**
 * The Twitch account changed: every draft, result, timer and remembered page
 * goes, so nothing typed or done as one account shows to the next. The
 * write counter keeps counting, so no old timer can match a new write.
 */
export function forgetAccount(): void {
  epoch += 1;
  for (const t of timers.values()) clearTimeout(t);
  timers.clear();
  entries.clear();
  lastPages.clear();
  notify();
}

/** Tests only: forget everything. */
export function resetManagerStore(): void {
  forgetAccount();
}

// ─── who manages here ─────────────────────────────────────────────────────────

export type Manages = 'yes' | 'no' | 'unknown';

/**
 * Whether the signed-in account manages this channel, for the Tools button and
 * the phone's Manage channel row, on desktop and phone alike. A moderator or
 * broadcaster badge says yes at once; an editor has no badge, so Rust's view
 * decides (`pages`: how many pages it lists, null until it lands for this
 * channel). A replay's chat offers moderators their chat tools only.
 */
export function managesHere(c: {
  twitch: boolean;
  signedIn: boolean;
  replay: boolean;
  modBadge: boolean;
  broadcaster: boolean;
  pages: number | null;
}): Manages {
  if (!c.twitch || !c.signedIn) return 'no';
  if (c.modBadge || c.broadcaster) return 'yes';
  if (c.replay) return 'no';
  if (c.pages === null) return 'unknown';
  return c.pages > 0 ? 'yes' : 'no';
}

// ─── which surface is open ────────────────────────────────────────────────────

/**
 * Whether a chat must close its stream manager panel, builder and window now:
 * the channel changed under them, the Twitch account changed (`was.account`
 * is the last known one), or the manager is known not to apply here. While
 * that is still unknown nothing closes. Either way nothing open may write on.
 */
export function mustCloseSurfaces(
  was: { channelId: string | null | undefined; account: string | null },
  now: { channelId: string | null | undefined; account: string | null; manages: Manages },
  anyOpen: boolean,
): boolean {
  if (was.channelId !== now.channelId) return true;
  if (was.account !== null && now.account !== null && was.account !== now.account) return true;
  return anyOpen && now.manages === 'no';
}

// ─── slash command events ─────────────────────────────────────────────────────

/** What a pane-scoped slash event (`streamnook-open-poll-composer`,
 *  `streamnook-clear-local-chat`) carries. */
export interface PaneEventDetail {
  /** The chat pane the command was typed in. */
  source?: string;
  channel?: string;
  kind?: string;
}

/**
 * Whether this pane (`mySource`, showing `myLogin`) acts on a slash event:
 * only the pane it was typed in, and only while it still shows that channel.
 * An event with no source (typed somewhere that does not say) is taken by the
 * pane showing its channel.
 */
export function acceptsComposerEvent(detail: PaneEventDetail | null | undefined, mySource: string, myLogin: string | null | undefined): boolean {
  const mine = (myLogin ?? '').toLowerCase();
  const theirs = (detail?.channel ?? '').toLowerCase();
  if (detail?.source) return detail.source === mySource && (!theirs || !mine || theirs === mine);
  return !!theirs && theirs === mine;
}

// ─── stream info edits ────────────────────────────────────────────────────────

/** A field's value as Stream info holds it. */
type InfoValue = StreamInfoDraft[InfoField];

/** What a sent field shows until Twitch's read has it, and the save that sent it. */
export type InfoLanding = Partial<Record<InfoField, { value: InfoValue; stamp: number }>>;

/** Stream info's entry in the store: the fields typed and not yet saved, a
 *  half-typed tag, and the fields a save sent that Twitch does not show yet. */
export interface InfoHeld {
  edits: Partial<StreamInfoDraft>;
  tagText: string;
  landing: InfoLanding;
}

export const INFO_FIELDS: readonly InfoField[] = ['title', 'category', 'tags', 'language'];

/** How long a sent field shows before Twitch's own value does again. */
export const LANDING_MS = 10_000;

/** Twitch's info as a draft. */
export function fromInfo(info: StreamInfoView | null): StreamInfoDraft {
  return {
    title: info?.title ?? '',
    category: info?.category ?? null,
    tags: info?.tags ?? [],
    language: info?.language ?? '',
  };
}

export function sameInfoValue(field: InfoField, a: InfoValue | undefined, b: InfoValue | undefined): boolean {
  switch (field) {
    case 'category': {
      const x = a as StreamInfoDraft['category'] | undefined;
      const y = b as StreamInfoDraft['category'] | undefined;
      return (x?.id ?? '') === (y?.id ?? '');
    }
    case 'tags': {
      const x = (a as string[] | undefined) ?? [];
      const y = (b as string[] | undefined) ?? [];
      return x.length === y.length && x.every((t, i) => t === y[i]);
    }
    default:
      return (a ?? '') === (b ?? '');
  }
}

/** What the form shows under the edits: Twitch's info, with sent fields that
 *  have not landed yet over it. */
export function infoUnderEdits(info: StreamInfoView | null, landing: InfoLanding): StreamInfoDraft {
  const out = fromInfo(info);
  for (const f of INFO_FIELDS) {
    const l = landing[f];
    if (l) (out as unknown as Record<InfoField, InfoValue>)[f] = l.value;
  }
  return out;
}

/** The whole form: what is under the edits, then the edits. */
export function shownInfo(info: StreamInfoView | null, held: InfoHeld | undefined): StreamInfoDraft {
  return { ...infoUnderEdits(info, held?.landing ?? {}), ...(held?.edits ?? {}) };
}

/** The edits for a form now showing `next`: only the fields that differ from
 *  what shows under them. A field typed back to that value is no edit. */
export function infoEdits(next: StreamInfoDraft, under: StreamInfoDraft): Partial<StreamInfoDraft> {
  const out: Partial<StreamInfoDraft> = {};
  for (const f of INFO_FIELDS) {
    if (!sameInfoValue(f, next[f], under[f])) (out as Record<InfoField, InfoValue>)[f] = next[f];
  }
  return out;
}

/**
 * After a save of `sent` (stamped `stamp`) went through: the sent fields leave
 * the edits and show as landing, except a field typed again since Save was
 * pressed, which stays an edit.
 */
export function landingAfterSave(held: InfoHeld, sent: Partial<StreamInfoDraft>, stamp: number): InfoHeld {
  const edits: Partial<StreamInfoDraft> = { ...held.edits };
  const landing: InfoLanding = { ...held.landing };
  for (const f of INFO_FIELDS) {
    if (!(f in sent)) continue;
    const value = sent[f] as InfoValue;
    landing[f] = { value, stamp };
    if (f in edits && sameInfoValue(f, edits[f], value)) delete edits[f];
  }
  return { ...held, edits, landing };
}

/** Landing fields Twitch's read now shows are dropped. */
export function landedInfo(held: InfoHeld, info: StreamInfoView | null): InfoHeld {
  if (!info) return held;
  const now = fromInfo(info);
  let changed = false;
  const landing: InfoLanding = { ...held.landing };
  for (const f of INFO_FIELDS) {
    const l = landing[f];
    if (l && sameInfoValue(f, l.value, now[f])) {
      delete landing[f];
      changed = true;
    }
  }
  return changed ? { ...held, landing } : held;
}

/**
 * A Stream info save of `sent` went through: its fields show as landing on
 * `key` until Twitch's read has them, or for `LANDING_MS`. Nothing is written
 * when the store was emptied for another account since the save began
 * (`epochAtSave`), so the next account never sees the last one's title.
 */
export function landInfoSave(key: string, sent: Partial<StreamInfoDraft>, epochAtSave: number): boolean {
  if (epochAtSave !== epoch) return false;
  const stamp = nextStamp();
  const cur = (getTool(key).draft as InfoHeld | undefined) ?? { edits: {}, tagText: '', landing: {} };
  replaceToolDraft(key, landingAfterSave(cur, sent, stamp));
  changeDraftLater(key, `${key}:landing:${stamp}`, LANDING_MS, (d) => landingExpired(d as InfoHeld, stamp));
  return true;
}

/** The save stamped `stamp` is over: its own landing fields go, and no other
 *  save's (a later save of the same field keeps showing). */
export function landingExpired(held: InfoHeld, stamp: number): InfoHeld {
  let changed = false;
  const landing: InfoLanding = { ...held.landing };
  for (const f of INFO_FIELDS) {
    if (landing[f]?.stamp === stamp) {
      delete landing[f];
      changed = true;
    }
  }
  return changed ? { ...held, landing } : held;
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
