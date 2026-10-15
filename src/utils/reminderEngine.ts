// The /remind typing aid in the desktop chat box: the next-word suggestions
// and the chip overlay over the field. Page UI only, synchronous on every
// keystroke. Parsing, saving, listing and posting reminders are Rust's
// (services/reminder_service.rs, through run_slash_command).

import type { CommandDefinition } from './chatCommands';
import type { Reminder } from '../types';

// How many back-to-back copies one reminder may post (Rust clamps to this too).
const MAX_REPEAT = 10;

// Duration accessors with back-compat: prefer the seconds field, fall back to
// the legacy *_minutes value.
function getIntervalSeconds(r: Reminder): number {
  return r.interval_seconds ?? (r.interval_minutes != null ? r.interval_minutes * 60 : 0);
}
function getDelaySeconds(r: Reminder): number {
  return r.delay_seconds ?? (r.delay_minutes != null ? r.delay_minutes * 60 : 0);
}
function getUptimeSeconds(r: Reminder): number {
  return r.uptime_seconds ?? (r.uptime_minutes != null ? r.uptime_minutes * 60 : 0);
}

/** Human-friendly duration, e.g. 30 -> "30s", 90 -> "1m30s", 1200 -> "20m". */
function formatDuration(totalSeconds: number): string {
  const total = Math.max(0, Math.round(totalSeconds));
  if (total < 60) return `${total}s`;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  return [h ? `${h}h` : '', m ? `${m}m` : '', sec ? `${sec}s` : ''].join('') || '0s';
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
