// Reminder posting.
//
// A reminder is a message StreamNook posts into a streamer's chat on your
// behalf, triggered by one of five "avenues": a repeating interval, a one-time
// delay after you join, a clock time, a stream-uptime threshold, or a keyword
// appearing in chat. WHEN one fires is Rust's (services/reminder_service.rs),
// which announces `reminders://fire`; the headless <ReminderEngine /> hands that
// to fireReminderFromRust, which expands the message and posts it, because the
// sent row belongs to this window's chat store. Reminders themselves are
// persisted in settings.reminders. The /remind parser lives here too.

import { useAppStore } from '../stores/AppStore';
import {
  useChatConnectionStore,
  sendChannelMessage,
  type SendUserInfo,
} from '../stores/chatConnectionStore';
import { expandUserCommand, formatStreamUptime, type TemplateContext, type CommandDefinition } from './chatCommands';
import { Logger } from './logger';
import type { Reminder } from '../types';

// Invisible Plane-14 tag character. Twitch's duplicate-message detector ignores
// it but the IRC pipeline accepts it, so appending a varying number of copies
// makes each repeat unique while staying visually identical. Same trick as the
// chat box's bypass-duplicate option.
const DUPLICATE_BYPASS_CHAR = '\u{E0000}';
// How many back-to-back copies a single reminder may post. The invisible-char
// bypass keeps Twitch from rejecting the repeats; the ceiling keeps a reminder
// from tripping Twitch's per-30s message rate limit.
const MAX_REPEAT = 10;
// Spacing between repeat copies so they land in order and read cleanly.
const REPEAT_SPACING_MS = 550;

/** Make copy `copyIndex` of a repeated send unique. Copy 0 is the plain text
 *  (so a single send looks exactly like a normal message); later copies carry
 *  an invisible suffix of increasing length. */
export function withDuplicateBypass(text: string, copyIndex: number): string {
  if (copyIndex <= 0) return text;
  return `${text} ${DUPLICATE_BYPASS_CHAR.repeat(copyIndex)}`;
}

export function clampRepeat(n: number | undefined): number {
  const v = Math.floor(Number(n));
  if (!Number.isFinite(v) || v < 1) return 1;
  return Math.min(v, MAX_REPEAT);
}

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

function makeId(): string {
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `rmd-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

// ── Target resolution + sending ─────────────────────────────────────────────

interface ReminderTarget {
  key: string; // channel login, lowercased — the chatConnectionStore key
  channelId: string | null;
  isCurrent: boolean; // whether this target is the channel you're actively watching
}

/** Post the reminder Rust says is due now, into the channel Rust resolved.
 *  A reminder deleted or disabled since is ignored. */
export async function fireReminderFromRust(id: string, channel: string, isCurrent: boolean): Promise<void> {
  const app = useAppStore.getState();
  const reminder = app.settings.reminders?.reminders?.find((r) => r.id === id);
  if (!reminder || !reminder.enabled) return;
  await fireReminder(reminder, {
    key: channel,
    channelId: isCurrent ? app.currentStream?.user_id ?? null : reminder.channel_id ?? null,
    isCurrent,
  });
}

function isConnected(key: string): boolean {
  return !!useChatConnectionStore.getState().channels.get(key)?.isConnected;
}

function buildContext(target: ReminderTarget): TemplateContext {
  const app = useAppStore.getState();
  const user = app.currentUser;
  // Stream placeholders only resolve for the channel you're actively watching;
  // a pinned, non-watched channel has no live data, so they come back empty.
  const stream = target.isCurrent ? app.currentStream : null;
  return {
    user_name: user?.display_name || user?.username || user?.login || '',
    user_id: user?.user_id || '',
    channel_name: stream?.user_name || stream?.user_login || target.key,
    channel_id: stream?.user_id || target.channelId || '',
    stream_title: stream?.title || '',
    stream_game: stream?.game_name || '',
    stream_uptime: formatStreamUptime(stream?.started_at),
    args: [],
  };
}

/** Post a reminder's message to its target channel, repeated `repeat_count`
 *  times with the invisible-character bypass so the copies aren't rejected. */
async function fireReminder(reminder: Reminder, target: ReminderTarget): Promise<void> {
  const app = useAppStore.getState();
  const user = app.currentUser;
  if (!user?.user_id) return;
  if (!isConnected(target.key)) return;

  const expanded = expandUserCommand(reminder.message, buildContext(target));
  const text = expanded.text.trim();
  if (!text) return;

  const userInfo: SendUserInfo = {
    username: user.login || user.username,
    displayName: user.display_name || user.username,
    userId: user.user_id,
    badges: '',
  };

  const copies = clampRepeat(reminder.repeat_count);
  for (let j = 0; j < copies; j++) {
    try {
      await sendChannelMessage(target.key, withDuplicateBypass(text, j), userInfo);
    } catch (err) {
      Logger.error('[Reminders] failed to send reminder:', err);
    }
    if (j < copies - 1) await delay(REPEAT_SPACING_MS);
  }
}

/** Fire a reminder's text immediately into the channel you're watching, without
 *  saving it. Backs `/remind now <message>`. */
export async function fireReminderNow(message: string, repeat: number): Promise<boolean> {
  const app = useAppStore.getState();
  const login = app.currentStream?.user_login?.toLowerCase();
  if (!login) return false;
  await fireReminder(
    {
      id: 'now',
      enabled: true,
      message,
      trigger: 'interval',
      repeat_count: clampRepeat(repeat),
      channel_scope: 'current',
    },
    { key: login, channelId: app.currentStream?.user_id ?? null, isCurrent: true },
  );
  return true;
}

// ── /remind quick-create parser ──────────────────────────────────────────────

export type RemindManageOp = 'remove' | 'enable' | 'disable' | 'toggle';

export interface RemindParseResult {
  list?: boolean;
  help?: boolean;
  clear?: boolean;
  manage?: { op: RemindManageOp; index: number };
  sendNow?: { message: string; repeat: number };
  reminder?: Reminder;
  summary?: string;
  error?: string;
}

/** Parse a duration into SECONDS. Accepts "30s", "20m", "2h", combos like
 *  "1m30s" / "1h30m", or a bare number (interpreted as minutes for back-compat
 *  with the original minutes-only grammar). */
function parseDurationSeconds(raw: string): number | null {
  const s = raw.trim().toLowerCase();
  if (!s) return null;
  if (/^\d+$/.test(s)) return parseInt(s, 10) * 60; // bare number = minutes
  const m = s.match(/^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/);
  if (m && (m[1] || m[2] || m[3])) {
    return (m[1] ? parseInt(m[1], 10) * 3600 : 0) + (m[2] ? parseInt(m[2], 10) * 60 : 0) + (m[3] ? parseInt(m[3], 10) : 0);
  }
  return null;
}

// Duration accessors with back-compat: prefer the seconds field, fall back to
// the legacy *_minutes value.
export function getIntervalSeconds(r: Reminder): number {
  return r.interval_seconds ?? (r.interval_minutes != null ? r.interval_minutes * 60 : 0);
}
export function getDelaySeconds(r: Reminder): number {
  return r.delay_seconds ?? (r.delay_minutes != null ? r.delay_minutes * 60 : 0);
}
export function getUptimeSeconds(r: Reminder): number {
  return r.uptime_seconds ?? (r.uptime_minutes != null ? r.uptime_minutes * 60 : 0);
}

/** Human-friendly duration, e.g. 30 -> "30s", 90 -> "1m30s", 1200 -> "20m". */
export function formatDuration(totalSeconds: number): string {
  const total = Math.max(0, Math.round(totalSeconds));
  if (total < 60) return `${total}s`;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  return [h ? `${h}h` : '', m ? `${m}m` : '', sec ? `${sec}s` : ''].join('') || '0s';
}

/** Parse "HH:MM" (24h) into a normalized "HH:MM" string, or null. */
function parseClock(raw: string): string | null {
  const m = raw.trim().match(/^(\d{1,2}):(\d{2})$/);
  if (!m) return null;
  const h = parseInt(m[1], 10);
  const min = parseInt(m[2], 10);
  if (h > 23 || min > 59) return null;
  return `${String(h).padStart(2, '0')}:${String(min).padStart(2, '0')}`;
}

const USAGE =
  'Usage: /remind every 20m <message> · in 30s · at 21:00 · uptime 2h · on <word> · now. Times accept s/m/h (30s, 90s, 1h30m). Add x3 to repeat. /remind list to manage, /remind help for everything.';

function parseIndex(raw: string | undefined): number | null {
  if (!raw) return null;
  const n = parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 1) return null;
  return n;
}

/** Short phrase for a reminder's trigger, e.g. "every 20m", "in 30s". */
function reminderTriggerPhrase(r: Reminder): string {
  switch (r.trigger) {
    case 'interval':
      return `every ${formatDuration(getIntervalSeconds(r))}`;
    case 'delay':
      return `in ${formatDuration(getDelaySeconds(r))}`;
    case 'clock':
      return `at ${r.clock_time}`;
    case 'uptime':
      return `at ${formatDuration(getUptimeSeconds(r))} uptime`;
    case 'keyword':
      return `on "${r.keyword}"`;
    default:
      return '';
  }
}

function describeReminder(reminder: Reminder): string {
  const repeat = clampRepeat(reminder.repeat_count);
  const times = repeat > 1 ? ` ×${repeat}` : '';
  const where =
    reminder.channel_scope === 'specific' && reminder.channel_login ? ` in #${reminder.channel_login}` : '';
  return `Reminder set: ${reminderTriggerPhrase(reminder)}${times}${where}`;
}

/** Lines for `/remind list` — a numbered roster the user manages by index. */
export function formatReminderList(reminders: Reminder[]): string[] {
  if (reminders.length === 0) {
    return ['No reminders yet. Example: /remind every 20m Plug the Discord — /remind help for all options.'];
  }
  const lines = reminders.map((r, i) => {
    const repeat = clampRepeat(r.repeat_count);
    const times = repeat > 1 ? ` ×${repeat}` : '';
    const where = r.channel_scope === 'specific' && r.channel_login ? `#${r.channel_login}` : 'current channel';
    const state = r.enabled ? 'on' : 'off';
    const msg = r.message.length > 60 ? `${r.message.slice(0, 57)}…` : r.message;
    return `#${i + 1} [${state}] ${reminderTriggerPhrase(r)}${times} · ${where} · "${msg}"`;
  });
  return [
    `Your reminders (${reminders.length}):`,
    ...lines,
    'Manage: /remind remove <#> · /remind disable <#> · /remind enable <#> · /remind clear',
  ];
}

// ── Guided autocomplete for the /remind flow ─────────────────────────────────
// Given the current input, suggest the next token so the command builds up step
// by step in the chat UI (subcommand → when → message, or which reminder to act
// on). Returned entries reuse CommandDefinition so the existing autocomplete
// popup renders them; `insertText`/`hint`/`badgeLabel` drive completion + labels.

export interface RemindFlowSuggestions {
  suggestions: CommandDefinition[];
  replaceFrom: number; // index in `value` where the current (partial) token starts
}

function mkStep(
  name: string,
  usage: string,
  description: string,
  badgeLabel: string,
): CommandDefinition {
  return { name, usage, description, category: 'Everyone', badgeLabel };
}

function mkHint(usage: string, description: string): CommandDefinition[] {
  return [{ name: 'hint', usage, description, category: 'Everyone', hint: true, badgeLabel: 'STEP' }];
}

function durationLabel(mode: string, d: string): string {
  if (mode === 'every') return `Repeat every ${d}`;
  if (mode === 'in') return `Once, ${d} from now`;
  return `Once live for ${d}`;
}

function reminderInlineLabel(r: Reminder): string {
  const msg = r.message.length > 40 ? `${r.message.slice(0, 37)}…` : r.message;
  return `${reminderTriggerPhrase(r)} · "${msg}"`;
}

// A completed repeat token, e.g. "x4" / "x:4" / "repeat:4". Mirrors the set the
// parser strips, so the flow can treat repeat counts as position-independent.
function isRepeatToken(t: string): boolean {
  return /^(?:x:?\d+|repeat:\d+)$/i.test(t);
}

// Quick-pick repeat counts surfaced in the flow. The user can also type any
// count up to MAX_REPEAT directly (e.g. x8).
function repeatPresets(): CommandDefinition[] {
  return [2, 3, 4, 5].map((n) => ({
    name: `x${n}`,
    usage: `x${n}`,
    description: `Send it ${n} times back to back`,
    category: 'Everyone' as const,
    badgeLabel: `×${n}`,
  }));
}

function filterRepeatPresets(partial: string): CommandDefinition[] {
  const lower = partial.toLowerCase();
  const matches = repeatPresets().filter((s) => s.name.toLowerCase().startsWith(lower));
  if (matches.length) return matches;
  return [{
    name: 'hint',
    usage: 'x<count>',
    description: `Send it up to ${MAX_REPEAT} times, e.g. x4`,
    category: 'Everyone',
    hint: true,
    badgeLabel: 'STEP',
  }];
}

// ── Command chip preview ─────────────────────────────────────────────────────
// Breaks a /remind line into labelled tokens so the chat UI can box each one as
// it solidifies (the command word, the subcommand, the "when", the repeat, the
// reminder index) and show the message tail plainly.

export type RemindSegmentKind = 'cmd' | 'verb' | 'when' | 'repeat' | 'index' | 'message' | 'space';
export interface RemindOverlaySegment {
  text: string;
  chip: boolean;
  kind: RemindSegmentKind;
}

const PREVIEW_VERBS = ['every', 'in', 'at', 'uptime', 'on', 'now', 'list', 'remove', 'delete', 'rm', 'del', 'enable', 'disable', 'toggle', 'clear', 'help'];
const WHEN_VERBS = ['every', 'in', 'at', 'uptime', 'on'];
const INDEX_VERBS = ['remove', 'delete', 'rm', 'del', 'enable', 'disable', 'toggle'];

// Break a /remind line into labelled segments that, concatenated, reproduce the
// input verbatim (whitespace preserved). The chat input renders these as a chip
// overlay aligned over the textarea, so each solidified option (the command, the
// subcommand, the "when", the repeat, the reminder index) shows as a little box
// while spaces and the free-text message stay plain.
export function tokenizeRemindOverlay(value: string): RemindOverlaySegment[] {
  const pieces = value.split(/(\s+)/); // keeps whitespace runs as their own pieces
  const segs: RemindOverlaySegment[] = [];
  let sawCmd = false;
  let mode = '';
  let whenTaken = false;
  let indexTaken = false;

  for (const piece of pieces) {
    if (piece === '') continue;
    if (/^\s+$/.test(piece)) {
      segs.push({ text: piece, chip: false, kind: 'space' });
      continue;
    }
    if (!sawCmd) {
      segs.push({ text: piece, chip: true, kind: 'cmd' });
      sawCmd = true;
      continue;
    }
    if (isRepeatToken(piece)) {
      segs.push({ text: piece, chip: true, kind: 'repeat' });
      continue;
    }
    if (!mode) {
      const v = piece.toLowerCase();
      if (PREVIEW_VERBS.includes(v)) {
        mode = v;
        segs.push({ text: piece, chip: true, kind: 'verb' });
      } else {
        // Shorthand with no verb: "/remind 20m ..." or "/remind 21:00 ...".
        mode = '_shorthand';
        whenTaken = true;
        segs.push({ text: piece, chip: true, kind: 'when' });
      }
      continue;
    }
    if (!whenTaken && WHEN_VERBS.includes(mode)) {
      segs.push({ text: piece, chip: true, kind: 'when' });
      whenTaken = true;
      continue;
    }
    if (!indexTaken && INDEX_VERBS.includes(mode)) {
      segs.push({ text: piece, chip: true, kind: 'index' });
      indexTaken = true;
      continue;
    }
    segs.push({ text: piece, chip: false, kind: 'message' });
  }

  return segs;
}

export function getRemindFlowSuggestions(value: string, reminders: Reminder[]): RemindFlowSuggestions | null {
  // Only active once we're past "/remind " (at least one space typed).
  if (!/^\/remind\s/i.test(value)) return null;

  const words = value.split(' ');
  const argWords = words.slice(1);
  const partial = argWords.length ? argWords[argWords.length - 1] : '';
  // Repeat tokens (x4 / repeat:4) can sit anywhere; treat them as transparent so
  // they don't shift which step the user is on, but remember one was given.
  const completedAll = argWords.slice(0, -1).filter((w) => w.length > 0);
  const completed = completedAll.filter((w) => !isRepeatToken(w));
  const hasRepeat = completedAll.some(isRepeatToken);
  const replaceFrom = value.length - partial.length;
  const p = partial.toLowerCase();
  const filt = (list: CommandDefinition[]) => list.filter((s) => s.name.toLowerCase().startsWith(p));

  const mode = completed.length > 0 ? completed[0].toLowerCase() : '';
  // True once the trigger's "when" has been supplied and we're on the message —
  // the point where a repeat count makes sense.
  const atMessageSlot =
    (mode === 'now' && completed.length >= 1) ||
    ((mode === 'every' || mode === 'in' || mode === 'at' || mode === 'uptime' || mode === 'on') && completed.length >= 2);

  // Currently typing a repeat token (x / x4 / repeat:4) at the message step.
  if (atMessageSlot && /^(?:x|repeat):?\d*$/i.test(partial)) {
    return { suggestions: filterRepeatPresets(partial), replaceFrom };
  }

  // Slot 1: which subcommand.
  if (completed.length === 0) {
    const verbs = [
      mkStep('every', 'every <minutes> <message>', 'Repeat on a timer', 'CREATE'),
      mkStep('in', 'in <minutes> <message>', 'Once, after a delay', 'CREATE'),
      mkStep('at', 'at <HH:MM> <message>', 'Once a day at a time', 'CREATE'),
      mkStep('uptime', 'uptime <duration> <message>', 'Once at a stream uptime', 'CREATE'),
      mkStep('on', 'on <keyword> <message>', 'When a keyword appears', 'CREATE'),
      mkStep('now', 'now <message>', 'Post it right now', 'CREATE'),
      mkStep('list', 'list', 'Show your reminders', 'MANAGE'),
      mkStep('remove', 'remove <#>', 'Delete one', 'MANAGE'),
      mkStep('enable', 'enable <#>', 'Turn one on', 'MANAGE'),
      mkStep('disable', 'disable <#>', 'Turn one off', 'MANAGE'),
      mkStep('clear', 'clear', 'Delete all of them', 'MANAGE'),
      mkStep('help', 'help', 'Full reference', 'MANAGE'),
    ];
    return { suggestions: filt(verbs), replaceFrom };
  }

  // Slot 2: when (duration) for every / in / uptime.
  if ((mode === 'every' || mode === 'in' || mode === 'uptime') && completed.length === 1) {
    const presets = ['30s', '1m', '5m', '10m', '15m', '20m', '30m', '1h', '2h'].map((d) =>
      mkStep(d, `${mode} ${d} <message>`, durationLabel(mode, d), 'WHEN'),
    );
    const matches = filt(presets);
    return {
      suggestions: matches.length ? matches : mkHint(`${mode} <time> <message>`, 'Type any length — 30s, 90s, 20m, 1h30m — then your message'),
      replaceFrom,
    };
  }

  // Slot 2: when (clock time) for at.
  if (mode === 'at' && completed.length === 1) {
    const presets = ['09:00', '12:00', '15:00', '18:00', '20:00', '21:00', '22:00'].map((t) =>
      mkStep(t, `at ${t} <message>`, 'Fires once a day at this time', 'WHEN'),
    );
    const matches = filt(presets);
    return {
      suggestions: matches.length ? matches : mkHint('at <HH:MM> <message>', 'Type a 24-hour time like 21:00, then your message'),
      replaceFrom,
    };
  }

  // Slot 2: keyword for on.
  if (mode === 'on' && completed.length === 1) {
    return { suggestions: mkHint('on <keyword> <message>', 'Type the word or phrase to watch for, then your message'), replaceFrom };
  }

  // Slot 2: which reminder, for the manage verbs.
  if (['remove', 'delete', 'rm', 'del', 'enable', 'disable', 'toggle'].includes(mode) && completed.length === 1) {
    if (reminders.length === 0) {
      return { suggestions: mkHint(`${mode} <#>`, 'No reminders yet — create one first'), replaceFrom };
    }
    const items = reminders.map((r, i) =>
      mkStep(String(i + 1), `${mode} ${i + 1}`, reminderInlineLabel(r), r.enabled ? 'ON' : 'OFF'),
    );
    return { suggestions: filt(items), replaceFrom };
  }

  // Final slot: the message body. While it's still empty, guide the user and
  // (until they've set a count) offer the repeat presets right here so "send it
  // N times" is part of the flow. Once they start typing the message, bow out so
  // @mention / emote autocomplete can take over.
  if (atMessageSlot) {
    if (p !== '') return null;
    const rows: CommandDefinition[] = [
      {
        name: 'hint',
        usage: '<message>',
        description: hasRepeat ? 'Type the message to post' : 'Type the message — or pick how many times to send it',
        category: 'Everyone',
        hint: true,
        badgeLabel: 'STEP',
      },
    ];
    if (!hasRepeat) rows.push(...repeatPresets());
    return { suggestions: rows, replaceFrom };
  }

  // list / help / clear take no further arguments — nothing to suggest.
  return null;
}

/** Lines for `/remind help` — the full command reference. */
export function remindHelpLines(): string[] {
  return [
    'Reminders auto-post a message into chat to nudge the streamer. Make one:',
    '/remind every 20m <message> — repeat every 20 minutes (also 30s, 90s, 1h30m)',
    '/remind in 30s <message> — once, 30 seconds from now',
    '/remind at 21:00 <message> — once a day at a set time (24h)',
    '/remind uptime 2h <message> — once when the stream has been live 2h',
    '/remind on <word> <message> — when that word shows up in chat',
    '/remind now <message> — post it right now (not saved)',
    'Times take s/m/h: 45s, 90s, 5m, 1h30m. Add x3 anywhere to post it 3 times.',
    'Manage: /remind list · /remind remove <#> · /remind enable <#> · /remind disable <#> · /remind clear',
  ];
}

/** Parse the arguments of a `/remind ...` command into an action. */
export function parseRemindCommand(
  args: string[],
  broadcasterLogin: string,
  broadcasterId: string,
): RemindParseResult {
  const tokens = args.filter((t) => t.length > 0);
  if (tokens.length === 0) return { list: true };

  // Pull an optional repeat token (x3 / x:3 / repeat:3) from anywhere in the line.
  let repeat = 1;
  const rest: string[] = [];
  for (const t of tokens) {
    const rm = t.match(/^x:?(\d+)$/i) || t.match(/^repeat:(\d+)$/i);
    if (rm) {
      repeat = clampRepeat(parseInt(rm[1], 10));
      continue;
    }
    rest.push(t);
  }
  if (rest.length === 0) return { error: USAGE };

  const base = (): Omit<Reminder, 'trigger'> => ({
    id: makeId(),
    enabled: true,
    message: '',
    repeat_count: repeat,
    // Typed in a streamer's chat, so pin it to that channel by default.
    channel_scope: broadcasterLogin ? 'specific' : 'current',
    channel_login: broadcasterLogin || undefined,
    channel_id: broadcasterId || undefined,
  });

  const mode = rest[0].toLowerCase();

  // Management subcommands (checked before the create verbs).
  if (mode === 'help' || mode === '?') return { help: true };
  if (mode === 'list' || mode === 'ls' || mode === 'all') return { list: true };
  if (mode === 'clear' || mode === 'clearall' || mode === 'removeall') return { clear: true };
  if (mode === 'remove' || mode === 'delete' || mode === 'rm' || mode === 'del') {
    const index = parseIndex(rest[1]);
    if (index === null) return { error: 'Which one? Run /remind list for the numbers, then e.g. /remind remove 2' };
    return { manage: { op: 'remove', index } };
  }
  if (mode === 'enable' || mode === 'disable' || mode === 'toggle') {
    const index = parseIndex(rest[1]);
    if (index === null) return { error: `Which one? Run /remind list, then e.g. /remind ${mode} 2` };
    return { manage: { op: mode, index } };
  }

  // /remind now <message> — fire immediately, don't save.
  if (mode === 'now') {
    const message = rest.slice(1).join(' ').trim();
    if (!message) return { error: 'Usage: /remind now <message>' };
    return { sendNow: { message, repeat } };
  }

  const finish = (reminder: Reminder, message: string): RemindParseResult => {
    if (!message.trim()) return { error: 'Add the message to post, e.g. /remind every 20m Plug the Discord' };
    reminder.message = message.trim();
    return { reminder, summary: describeReminder(reminder) };
  };

  switch (mode) {
    case 'every': {
      const secs = parseDurationSeconds(rest[1] || '');
      if (secs === null || secs < 1) return { error: 'How often? e.g. /remind every 20m or every 30s <message>' };
      return finish({ ...base(), trigger: 'interval', interval_seconds: secs }, rest.slice(2).join(' '));
    }
    case 'in': {
      const secs = parseDurationSeconds(rest[1] || '');
      if (secs === null || secs < 1) return { error: 'How long from now? e.g. /remind in 30s or in 5m <message>' };
      return finish({ ...base(), trigger: 'delay', delay_seconds: secs }, rest.slice(2).join(' '));
    }
    case 'at': {
      const clock = parseClock(rest[1] || '');
      if (!clock) return { error: 'What time? Use 24-hour HH:MM, e.g. /remind at 21:00 <message>' };
      return finish({ ...base(), trigger: 'clock', clock_time: clock }, rest.slice(2).join(' '));
    }
    case 'uptime': {
      const secs = parseDurationSeconds(rest[1] || '');
      if (secs === null || secs < 1) return { error: 'At what uptime? e.g. /remind uptime 2h or 90s <message>' };
      return finish({ ...base(), trigger: 'uptime', uptime_seconds: secs }, rest.slice(2).join(' '));
    }
    case 'on': {
      const keyword = rest[1] || '';
      if (!keyword) return { error: 'On what word? e.g. /remind on giveaway <message>' };
      return finish(
        { ...base(), trigger: 'keyword', keyword, keyword_match: 'contains', keyword_from: 'anyone', keyword_cooldown_minutes: 5 },
        rest.slice(2).join(' '),
      );
    }
    default: {
      // Shorthands with no leading verb: "/remind 20m <message>" and
      // "/remind 21:00 <message>".
      const clock = parseClock(mode);
      if (clock) return finish({ ...base(), trigger: 'clock', clock_time: clock }, rest.slice(1).join(' '));
      const secs = parseDurationSeconds(mode);
      if (secs !== null && secs >= 1) {
        return finish({ ...base(), trigger: 'interval', interval_seconds: secs }, rest.slice(1).join(' '));
      }
      return { error: USAGE };
    }
  }
}
