// Mass-action moderation engine for the /nuke and /undo commands.
//
// /nuke <pattern> <action> <past[:future]>
//   pattern: plain phrase (case-insensitive substring) OR /regex/flags
//   action: delete | ban | <duration>   (where duration is e.g. 10m, 1h, 5s)
//   past:   how far back to scan the local message buffer (e.g. 5m)
//   future: optional, how long to keep matching new messages and applying the
//           same action. Without :future, only past-window matches are nuked.
//
// /undo reverses the most recent nuke per channel by unbanning the affected
// user ids. Deletes are noted but cannot be reversed by Twitch.

import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { useChatConnectionStore, injectSystemMessage } from '../stores/chatConnectionStore';
import { Logger } from './logger';
import type { BackendChatMessage } from '../services/twitchChat';

export type NukeAction =
  | { kind: 'delete' }
  | { kind: 'ban' }
  | { kind: 'timeout'; seconds: number };

export interface ParsedNuke {
  pattern: RegExp;
  patternSource: string;
  action: NukeAction;
  pastSeconds: number;
  futureSeconds: number;
}

export interface ParseError {
  error: string;
}

const UNITS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86400 };

function parseDurationToken(tok: string): number | null {
  const m = tok.match(/^(\d+)([smhd])$/);
  if (!m) return null;
  const unit = UNITS[m[2]];
  return unit ? Number(m[1]) * unit : null;
}

function parseWindow(tok: string): { past: number; future: number } | null {
  const m = tok.match(/^(\d+[smhd])(?::(\d+[smhd]))?$/);
  if (!m) return null;
  const past = parseDurationToken(m[1]);
  if (past === null) return null;
  const future = m[2] ? parseDurationToken(m[2]) : 0;
  if (future === null) return null;
  return { past, future };
}

function parseActionToken(tok: string): NukeAction | null {
  if (tok === 'delete') return { kind: 'delete' };
  if (tok === 'ban') return { kind: 'ban' };
  const secs = parseDurationToken(tok);
  if (secs !== null) return { kind: 'timeout', seconds: secs };
  return null;
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function parsePattern(patternStr: string): { regex: RegExp; source: string } | { error: string } {
  // /regex/flags form
  const m = patternStr.match(/^\/(.+)\/([gimuy]*)$/);
  if (m) {
    try {
      // `g` and `y` make `.test()` resume from the last match (lastIndex), so a
      // pattern reused across messages would skip real matches. Matching is
      // per message; neither flag means anything here.
      const flags = m[2].replace(/[gy]/g, '');
      return { regex: new RegExp(m[1], flags || 'i'), source: patternStr };
    } catch (err: unknown) {
      return { error: `invalid regex: ${err instanceof Error ? err.message : String(err)}` };
    }
  }
  // literal substring, case-insensitive
  try {
    return { regex: new RegExp(escapeRegex(patternStr), 'i'), source: patternStr };
  } catch (err: unknown) {
    return { error: `invalid pattern: ${err instanceof Error ? err.message : String(err)}` };
  }
}

export function parseNukeArgs(args: string): ParsedNuke | ParseError {
  const tokens = args.trim().split(/\s+/).filter(Boolean);
  if (tokens.length < 3) {
    return { error: 'usage: /nuke <pattern> <action> <past[:future]>  e.g. /nuke spam ban 5m:1m' };
  }
  // Parse from the right: last = window, second-to-last = action, rest = pattern.
  const window = parseWindow(tokens[tokens.length - 1]);
  if (!window) {
    return { error: `last argument must be a window like "5m" or "5m:1m" — got "${tokens[tokens.length - 1]}"` };
  }
  const action = parseActionToken(tokens[tokens.length - 2]);
  if (!action) {
    return { error: `second-to-last argument must be "delete", "ban", or a duration like "10m" — got "${tokens[tokens.length - 2]}"` };
  }
  const patternStr = tokens.slice(0, tokens.length - 2).join(' ').trim();
  if (!patternStr) return { error: 'pattern is required' };
  const patternResult = parsePattern(patternStr);
  if ('error' in patternResult) return patternResult;
  return {
    pattern: patternResult.regex,
    patternSource: patternResult.source,
    action,
    pastSeconds: window.past,
    futureSeconds: window.future,
  };
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

interface NukeAffected {
  user_id: string;
  username: string;
  message_ids: string[];
}

interface NukeRecord {
  ranAt: number;
  channel: string;
  broadcasterId: string;
  parsed: ParsedNuke;
  affected: NukeAffected[];
  futureExpiresAt: number; // 0 if no future window
  seenUserIds: Set<string>; // for future-window dedupe
  // Handle for the window-close timer, so `/nuke stop` can cancel it instead of
  // leaving it to fire a "window closed" notice for a nuke that already ended.
  closeTimer?: ReturnType<typeof setTimeout>;
}

// Per-channel last nuke. /undo pops the most recent.
const lastNukeByChannel = new Map<string, NukeRecord>();
// Active future-window nukes; chatConnectionStore checks this on every PRIVMSG.
const activeNukes = new Map<string, NukeRecord[]>(); // channel -> records

export function getActiveNukesForChannel(channel: string): NukeRecord[] {
  return activeNukes.get(channel.toLowerCase()) ?? [];
}

function pickStructured(msg: string | BackendChatMessage): BackendChatMessage | null {
  if (typeof msg === 'string') return null; // raw IRC form, ignore for nuke matching
  return msg;
}

/** The reason recorded against a nuke's bans and timeouts. Shown to the user in
 *  Twitch's own mod view, so it's worth letting people write their own. */
function nukeReason(): string {
  const custom = useAppStore.getState().settings.moderation?.nuke_reason?.trim();
  return custom || '/nuke';
}

async function applyActionToUser(
  broadcasterId: string,
  affected: NukeAffected,
  action: NukeAction,
): Promise<void> {
  try {
    if (action.kind === 'delete') {
      for (const msgId of affected.message_ids) {
        await invoke('delete_chat_message', { broadcasterId, messageId: msgId });
      }
    } else if (action.kind === 'ban') {
      await invoke('ban_user', {
        broadcasterId,
        targetUserId: affected.user_id,
        duration: null,
        reason: nukeReason(),
      });
    } else {
      await invoke('ban_user', {
        broadcasterId,
        targetUserId: affected.user_id,
        duration: action.seconds,
        reason: nukeReason(),
      });
    }
  } catch (err) {
    Logger.error(`[nuke] action failed for ${affected.username}:`, err);
  }
}

export interface NukePreview {
  matchedMessages: number;
  affected: NukeAffected[];
}

/**
 * Everything /nuke WOULD hit right now, with no side effects.
 *
 * Shared with executeNuke so the count shown while you type is produced by the
 * same code that later does the banning. A preview that could disagree with the
 * action would be worse than no preview.
 */
export function previewNuke(channel: string, parsed: ParsedNuke): NukePreview {
  const slice = useChatConnectionStore.getState().channels.get(channel.toLowerCase());
  if (!slice) return { matchedMessages: 0, affected: [] };

  const cutoffMs = Date.now() - parsed.pastSeconds * 1000;
  const grouped = new Map<string, NukeAffected>();
  let matchedMessages = 0;

  for (const msg of slice.messages) {
    const structured = pickStructured(msg);
    if (!structured) continue;
    if (!structured.user_id || !structured.id) continue;
    if (structured.user_id === 'tw-system') continue; // skip our own system rows

    // Timestamp filter — fall back to "include" if timestamp missing.
    const ts = structured.timestamp ? Date.parse(structured.timestamp) : NaN;
    if (!Number.isNaN(ts) && ts < cutoffMs) continue;

    if (!parsed.pattern.test(structured.content)) continue;

    matchedMessages++;
    const existing = grouped.get(structured.user_id);
    if (existing) {
      existing.message_ids.push(structured.id);
    } else {
      grouped.set(structured.user_id, {
        user_id: structured.user_id,
        username: structured.display_name || structured.username,
        message_ids: [structured.id],
      });
    }
  }

  return { matchedMessages, affected: [...grouped.values()] };
}

/**
 * Cancel any armed future windows for a channel. Returns how many were stopped.
 */
export function stopActiveNukes(channel: string): number {
  const chKey = channel.toLowerCase();
  const records = activeNukes.get(chKey) ?? [];
  if (records.length === 0) return 0;
  for (const record of records) {
    if (record.closeTimer !== undefined) clearTimeout(record.closeTimer);
    record.futureExpiresAt = 0;
  }
  activeNukes.delete(chKey);
  return records.length;
}

export function describeAction(action: NukeAction): string {
  if (action.kind === 'delete') return 'delete';
  if (action.kind === 'ban') return 'ban';
  return `timeout ${action.seconds}s`;
}

/**
 * Execute /nuke against the past window of the current channel's local buffer.
 * Optionally arms a future-window subscription.
 */
export async function executeNuke(
  channel: string,
  broadcasterId: string,
  parsed: ParsedNuke,
): Promise<{ matchedMessages: number; affectedUsers: number }> {
  const chKey = channel.toLowerCase();
  const slice = useChatConnectionStore.getState().channels.get(chKey);
  if (!slice) {
    injectSystemMessage(chKey, '/nuke: no chat connected for this channel.');
    return { matchedMessages: 0, affectedUsers: 0 };
  }

  const { matchedMessages, affected } = previewNuke(chKey, parsed);

  // Execute concurrently per user. Tauri handle invoke is async; the Rust side
  // serializes Helix-bound mod actions naturally, so we don't bother throttling
  // ourselves here. If 429s become a problem, batch in groups of N with a tick.
  await Promise.all(affected.map((a) => applyActionToUser(broadcasterId, a, parsed.action)));

  const record: NukeRecord = {
    ranAt: Date.now(),
    channel: chKey,
    broadcasterId,
    parsed,
    affected,
    futureExpiresAt: parsed.futureSeconds > 0 ? Date.now() + parsed.futureSeconds * 1000 : 0,
    seenUserIds: new Set(affected.map((a) => a.user_id)),
  };
  lastNukeByChannel.set(chKey, record);

  if (parsed.futureSeconds > 0) {
    const initialAffectedCount = affected.length;
    const list = activeNukes.get(chKey) ?? [];
    list.push(record);
    activeNukes.set(chKey, list);
    record.closeTimer = setTimeout(() => {
      const cur = activeNukes.get(chKey) ?? [];
      const next = cur.filter((r) => r !== record);
      if (next.length) activeNukes.set(chKey, next);
      else activeNukes.delete(chKey);
      const extra = Math.max(0, record.affected.length - initialAffectedCount);
      injectSystemMessage(
        chKey,
        `/nuke window closed for pattern "${parsed.patternSource}". Caught ${extra} additional user(s) during the future window.`,
      );
    }, parsed.futureSeconds * 1000);
  }

  injectSystemMessage(
    chKey,
    `/nuke pattern "${parsed.patternSource}" → ${describeAction(parsed.action)} on ${matchedMessages} message(s) from ${affected.length} user(s)${
      parsed.futureSeconds > 0 ? `. Future window: ${parsed.futureSeconds}s.` : '.'
    }`,
  );

  return { matchedMessages, affectedUsers: affected.length };
}

/**
 * Check a freshly-arrived chat message against any active future-window nukes
 * for the channel and execute the matching action. Called by the PRIVMSG path
 * in chatConnectionStore.
 */
export async function checkActiveNukesForMessage(
  channel: string,
  msg: BackendChatMessage,
): Promise<void> {
  const records = activeNukes.get(channel.toLowerCase());
  if (!records || !records.length) return;
  if (!msg.user_id || msg.user_id === 'tw-system') return;
  if (!msg.content) return;

  const now = Date.now();
  for (const record of records) {
    if (now > record.futureExpiresAt) continue;
    if (!record.parsed.pattern.test(msg.content)) continue;
    // Avoid double-acting on the same user within one nuke
    const alreadySeen = record.seenUserIds.has(msg.user_id);
    record.seenUserIds.add(msg.user_id);
    const affected: NukeAffected = {
      user_id: msg.user_id,
      username: msg.display_name || msg.username,
      message_ids: [msg.id],
    };
    if (record.parsed.action.kind === 'delete') {
      // For delete action, always delete the new message even if user was already banned
      await applyActionToUser(record.broadcasterId, affected, record.parsed.action);
    } else if (!alreadySeen) {
      // For ban/timeout, only act once per user per nuke
      record.affected.push(affected);
      await applyActionToUser(record.broadcasterId, affected, record.parsed.action);
    }
  }
}

/**
 * Reverse the most recent nuke on this channel. Bans and timeouts are
 * unbanned via the existing unban_user Tauri command; deletes are noted
 * but cannot be undone.
 */
export async function executeUndo(channel: string): Promise<void> {
  const chKey = channel.toLowerCase();
  const record = lastNukeByChannel.get(chKey);
  if (!record) {
    injectSystemMessage(chKey, '/undo: no nuke to undo on this channel.');
    return;
  }
  lastNukeByChannel.delete(chKey);
  // Also drop any active future-window for this record
  const cur = activeNukes.get(chKey) ?? [];
  const filtered = cur.filter((r) => r !== record);
  if (filtered.length) activeNukes.set(chKey, filtered);
  else activeNukes.delete(chKey);

  if (record.parsed.action.kind === 'delete') {
    injectSystemMessage(
      chKey,
      `/undo: cannot restore ${record.affected.reduce((n, a) => n + a.message_ids.length, 0)} deleted message(s). Twitch does not allow message un-delete.`,
    );
    return;
  }

  let unbanned = 0;
  for (const a of record.affected) {
    try {
      await invoke('unban_user', { broadcasterId: record.broadcasterId, targetUserId: a.user_id });
      unbanned++;
    } catch (err) {
      Logger.error(`[undo] failed to unban ${a.username}:`, err);
    }
  }
  injectSystemMessage(chKey, `/undo reversed nuke on "${record.parsed.patternSource}" — unbanned ${unbanned}/${record.affected.length} user(s).`);
}

/**
 * Quick check used by the slash handler to short-circuit if the user isn't a
 * moderator (or broadcaster) of the current channel. The user-badges cache
 * in chatConnectionStore is the source of truth — same string format the
 * existing ModeratorMenu mounting check uses.
 */
export function isUserModeratorOf(channel: string): boolean {
  const chKey = channel.toLowerCase();
  const slice = useChatConnectionStore.getState().channels.get(chKey);
  const badges = slice?.userBadges ?? '';
  // userBadges is a comma-separated list like "moderator/1,subscriber/12"
  if (!badges) return false;
  return /\bmoderator\/|\bbroadcaster\/|\bglobal_mod\//.test(badges);
}

// Test-only export for the parser. Lets a unit test cover parseNukeArgs
// without dragging in the rest of the engine.
export const __testing = { parseNukeArgs, parseDurationToken, parseWindow, parseActionToken, parsePattern };
