// Account automations as the settings page, the chat dots and the command
// palette read them. Rust owns every rule, check and write
// (commands/account_automations.rs). This file names what those commands take
// and answer, wraps them, and keeps the small pure helpers the surfaces share,
// so each is tested once.

import { invoke } from '@tauri-apps/api/core';
import type {
  AutomationAccountRef,
  AutomationAction,
  AutomationAway,
  AutomationEventKind,
  AutomationFieldError,
  AutomationLine,
  AutomationRule,
  AutomationRunState,
  AutomationSenderRole,
  AutomationTrigger,
} from '../types';
import { makeKey, parseKey } from './providerKey';

/** Each Activity entry, sent to the windows watching the log (`emit_to`). */
export const AUTOMATION_ACTIVITY_EVENT = 'automations://activity';
/** A Notify action's toast and sound, sent to the windows that could play it (`emit_to`). */
export const AUTOMATION_NOTIFY_EVENT = 'automations://notify';
/** The status, announced to every window when it changes. */
export const AUTOMATION_STATUS_EVENT = 'automations://status';

/** Activity entries read per page. */
export const ACTIVITY_PAGE = 100;
/** Entries one view keeps (Rust keeps 500). */
const ACTIVITY_KEPT = 500;
/** What away says when no message was ever written. */
export const DEFAULT_AWAY_MESSAGE = 'Back soon';

/** A signed-in account the pickers offer (7.3a `AccountChoice`). */
export interface AutomationAccount {
  id: string;
  login: string;
  /** The account the slot token was last verified as (shown as "main account"). */
  main: boolean;
}

/** One stored rule, with what this device knows about it (7.3a `RuleStatus`). */
export interface AutomationRuleStatus {
  id: string;
  name: string;
  enabled: boolean;
  live: boolean;
  /** False for a rule a newer StreamNook made, which shows read-only. */
  readable: boolean;
  /** The typed rule; null exactly when `readable` is false. */
  rule: AutomationRule | null;
  /** One line Rust writes about what the rule does. */
  summary: string;
  /** Times it acted (Live). */
  count: number;
  last_fire_ms: number | null;
  /** Why it cannot run right now, in your words. */
  held: string | null;
  /** How its last try ended ("Another device answered this"). */
  last_outcome: string | null;
}

/** `automations_status`, and the `automations://status` event. */
export interface AutomationStatus {
  run: AutomationRunState;
  /** What Resume returns to; never 'off'. */
  last_active: AutomationRunState;
  away: AutomationAway | null;
  last_away_message: string;
  device_name: string;
  rules: AutomationRuleStatus[];
  /** Chats the rules keep joined to listen in. */
  chats_kept: string[];
  /** Composite keys (`twitch:login`) of chats a Live rule with an outward action covers. */
  live_chats: string[];
  /** A Live rule with an outward action covers any chat you open. */
  live_any: boolean;
  /** Three claims in a row went unanswered: rules wait. */
  outage: boolean;
  /** StreamNook asked rules to wait a moment. */
  busy: boolean;
  accounts: AutomationAccount[];
  /** This device runs rules right now. */
  foreground: boolean;
}

/** `automations_save_rule` and `automations_add_template` (7.3a `SaveAnswer`). */
export interface AutomationSaveResult {
  /** The rule as stored; with errors, what was tried. */
  rule: AutomationRule;
  errors: AutomationFieldError[];
  /** False when there were errors: nothing was written. */
  saved: boolean;
}

/** 7.2 `TemplateView`. */
export interface AutomationTemplate {
  id: string;
  name: string;
  description: string;
  /** The rule's summary line. */
  summary: string;
}

/** Where a variable may go (7.2 `VariableScope`). */
export type AutomationVariableScope = 'any' | 'own_channel' | 'chatter' | 'badge';

/** 7.2 `VariableRow`. */
export interface AutomationVariable {
  /** Without braces: `['user', 'user.login', 'user.id']`; the first is inserted. */
  names: string[];
  /** What it fills in. */
  value: string;
  allowed: string;
  scope: AutomationVariableScope;
}

export interface AutomationBadgeSample {
  feed_id: string;
  name: string;
  phase: 'announced' | 'earnable';
}

export interface AutomationTestEvent {
  kind: AutomationEventKind;
  months: number;
  gifts: number;
  viewers: number;
  bits: number;
}

/** What Try it and the editor's preview ask about. */
export interface AutomationTestSample {
  text: string;
  /** A Twitch login. */
  channel: string;
  /** The sender's login. */
  sender: string;
  roles: AutomationSenderRole[];
  first_message: boolean;
  event: AutomationTestEvent | null;
  /** A recent drop; null with a badge rule means the newest one. */
  badge_feed_id: string | null;
  /** An unsaved rule to try instead of the stored ones. Its texts come back
   *  even when it would not act, so the editor can preview them. */
  draft: AutomationRule | null;
}

export interface AutomationTestText {
  action_id: string;
  kind: AutomationAction['kind'];
  text: string;
}

export interface AutomationTestRule {
  rule_id: string;
  name: string;
  fired: boolean;
  /** Each check, in order, in your words. */
  trace: string[];
  texts: AutomationTestText[];
  /** A badge rule's filled link for the drop tried. */
  link: string | null;
  errors: AutomationFieldError[];
}

export interface AutomationTestResult {
  rules: AutomationTestRule[];
}

export type AutomationActivityKind = 'acted' | 'test' | 'skipped' | 'notice';
export type AutomationActivityFilter = 'all' | 'acted' | 'test' | 'skipped';

/** 7.2 `activity::Entry`; empty strings mean none. */
export interface AutomationActivityEntry {
  id: number;
  /** The latest time (a folded skip's last repeat). */
  at_ms: number;
  first_at_ms: number;
  kind: AutomationActivityKind;
  /** Empty for a notice about automations as a whole. */
  rule_id: string;
  rule_name: string;
  /** Composite key of the chat, or empty. */
  channel: string;
  text: string;
  /** Times it happened; more than 1 only for folded skips. */
  count: number;
  /** The id this entry had before it folded into another and moved to the top with a new id; 0 when it did not fold. */
  replaces: number;
}

/** The `automations://activity` payload (batched, 7.3a decision 14). */
export interface AutomationActivityBatch {
  entries: AutomationActivityEntry[];
}

/** The `automations://notify` payload. */
export interface AutomationNotice {
  id: number;
  title: string;
  body: string;
  sound: string | null;
  /** 0 to 1. */
  volume: number;
  toast: boolean;
  /** Composite key of the chat, or empty. */
  channel: string;
}

/** The automations commands, one wrapper each. */
export const automationsApi = {
  status: () => invoke<AutomationStatus>('automations_status'),
  saveRule: (rule: AutomationRule, expectedUpdatedMs: number) =>
    invoke<AutomationSaveResult>('automations_save_rule', { rule, expectedUpdatedMs }),
  deleteRule: (id: string) => invoke<void>('automations_delete_rule', { id }),
  moveRule: (id: string, beforeId: string | null) => invoke<void>('automations_move_rule', { id, beforeId }),
  setRuleFlags: (id: string, flags: { enabled?: boolean; live?: boolean }) =>
    invoke<void>('automations_set_rule_flags', { id, enabled: flags.enabled ?? null, live: flags.live ?? null }),
  setRun: (state: AutomationRunState) => invoke<void>('automations_set_run', { state }),
  setAway: (message: string | null) => invoke<void>('automations_set_away', { message }),
  test: (sample: AutomationTestSample) => invoke<AutomationTestResult>('automations_test', { sample }),
  badgeSamples: () => invoke<AutomationBadgeSample[]>('automations_badge_samples'),
  templates: () => invoke<AutomationTemplate[]>('automations_templates'),
  addTemplate: (id: string) => invoke<AutomationSaveResult>('automations_add_template', { id }),
  variables: () => invoke<AutomationVariable[]>('automations_variables'),
  activity: (filter: AutomationActivityFilter, limit: number, before: number | null) =>
    invoke<AutomationActivityEntry[]>('automations_activity', { filter, limit, before }),
  clearActivity: () => invoke<void>('automations_clear_activity'),
  watchActivity: (watch: boolean) => invoke<void>('automations_watch_activity', { watch }),
  setDeviceName: (name: string) => invoke<string>('automations_set_device_name', { name }),
  /** True when the link answers; the words under Check it are the clients' own (`LINK_READY`, `LINK_NOT_READY`). */
  checkLink: (url: string) => invoke<boolean>('automations_check_link', { url }),
};

/** What Check it says; the iPhone says the same two lines. */
export const LINK_READY = 'The link is ready.';
export const LINK_NOT_READY = "The link isn't ready yet.";

let variables: Promise<AutomationVariable[]> | null = null;

/** The variable table, read once per window (it never changes while the app runs). */
export function loadAutomationVariables(): Promise<AutomationVariable[]> {
  variables ??= automationsApi.variables().catch(() => {
    variables = null;
    return [];
  });
  return variables;
}

/** What a failed command says, for a line under the control that made it. */
export function errorText(e: unknown): string {
  if (typeof e === 'string' && e.trim()) return e;
  if (e instanceof Error && e.message) return e.message;
  return "That didn't go through. Try again.";
}

/** The away message to set: what was typed, else the default. */
export const awayMessageOr = (typed: string): string => typed.trim() || DEFAULT_AWAY_MESSAGE;

/** Resume: back to Test or On, whichever was on before the pause. */
export async function resumeAutomations(): Promise<AutomationRunState> {
  const status = await automationsApi.status();
  const state: AutomationRunState = status.last_active === 'off' ? 'test' : status.last_active;
  await automationsApi.setRun(state);
  return state;
}

/** Set away on this device with the last message written. */
export async function setAwayNow(): Promise<void> {
  const status = await automationsApi.status();
  await automationsApi.setAway(awayMessageOr(status.last_away_message));
}

// ---------- The chat dot ----------------------------------------------------

/** On: a Live rule can act in this chat. Waiting: it would, but StreamNook cannot be reached. */
export type AutomationDotState = 'on' | 'waiting';

/** A chat key in `makeKey`'s form, whatever form it came in. */
export function chatKey(key: string): string {
  const { provider, channel } = parseKey(key);
  return makeKey(provider, channel);
}

/** Whether a chat showing these rooms wears the dot. A combined chat passes
 *  every member room and wears it when any one is covered. */
export function dotStateFor(
  status: Pick<AutomationStatus, 'run' | 'live_chats' | 'live_any' | 'outage'> | null,
  keys: readonly string[],
): AutomationDotState | null {
  if (!status || status.run !== 'on' || keys.length === 0) return null;
  const live = new Set(status.live_chats.map(chatKey));
  if (!status.live_any && !keys.some((k) => live.has(chatKey(k)))) return null;
  return status.outage ? 'waiting' : 'on';
}

/** `#login` for a chat key. */
export const chatLabel = (key: string): string => `#${parseKey(key).channel}`;

// ---------- Activity --------------------------------------------------------

function dayStart(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Entries from today (local time) and from before. Order is kept. */
export function groupActivity(
  entries: readonly AutomationActivityEntry[],
  now: number,
): { today: AutomationActivityEntry[]; earlier: AutomationActivityEntry[] } {
  const today = dayStart(now);
  return {
    today: entries.filter((e) => e.at_ms >= today),
    earlier: entries.filter((e) => e.at_ms < today),
  };
}

export const inFilter = (entry: AutomationActivityEntry, filter: AutomationActivityFilter): boolean =>
  filter === 'all' || entry.kind === filter;

/** A live entry put at the top of the shown list: one this filter hides is
 *  left out, and the older row a folded skip replaces (`replaces`, its old id)
 *  is removed, so the line never shows twice. */
export function applyEntry(
  list: readonly AutomationActivityEntry[],
  entry: AutomationActivityEntry,
  filter: AutomationActivityFilter,
): AutomationActivityEntry[] {
  if (!inFilter(entry, filter)) return [...list];
  return [entry, ...list.filter((e) => e.id !== entry.id && (entry.replaces === 0 || e.id !== entry.replaces))].slice(0, ACTIVITY_KEPT);
}

// ---------- The editor ------------------------------------------------------

/** The first error at exactly this field. */
export const errorAt = (errors: readonly AutomationFieldError[], path: string): string | null =>
  errors.find((e) => e.path === path)?.message ?? null;

/** Every error at this field or inside it. */
export const errorsUnder = (errors: readonly AutomationFieldError[], path: string): AutomationFieldError[] =>
  errors.filter((e) => e.path === path || e.path.startsWith(`${path}.`) || e.path.startsWith(`${path}[`));

/** An element id in Rust's own form (`a-`, `l-`, `c-` and 8 hex), so a new
 *  action, line or chat has a stable key while it is edited. Rust keeps an id
 *  it is given and fills only empty or repeated ones. */
export function newElementId(prefix: 'a' | 'l' | 'c'): string {
  const n = new Uint32Array(1);
  crypto.getRandomValues(n);
  return `${prefix}-${n[0].toString(16).padStart(8, '0')}`;
}

export const DEFAULT_LIMITS = {
  rule_cooldown_seconds: 30,
  user_cooldown_seconds: 300,
  max_per_hour: 30,
  max_wait_seconds: 10,
};

export type TriggerKind = AutomationTrigger['kind'];

export const newLine = (text = ''): AutomationLine => ({ id: newElementId('l'), text });

export function actionOf(kind: AutomationAction['kind']): AutomationAction {
  switch (kind) {
    case 'reply':
      return { kind, id: newElementId('a'), texts: [newLine()], pick: 'random', thread: true };
    case 'send':
      return { kind, id: newElementId('a'), to: null, texts: [newLine()], pick: 'random' };
    case 'notify':
      return { kind, id: newElementId('a'), title: '', body: '', toast: true, system: false, sound: null, volume: 1 };
    case 'command':
      return { kind, id: newElementId('a'), line: '/slow 30' };
    case 'whisper':
      return { kind, id: newElementId('a'), to: null, texts: [newLine()], pick: 'random' };
    case 'timeout':
      return { kind, id: newElementId('a'), seconds: 600, reason: '' };
    case 'delete':
    case 'shoutout':
      return { kind, id: newElementId('a') };
    case 'pin':
      return { kind, id: newElementId('a'), seconds: null };
    case 'announce':
      return { kind, id: newElementId('a'), text: '', color: 'primary' };
    case 'join_chat':
    case 'leave_chat':
      return { kind, id: newElementId('a'), channel: { id: newElementId('c'), key: '', room_id: '', name: '' } };
    case 'wait':
      return { kind, id: newElementId('a'), seconds: 5 };
    case 'log':
      return { kind, id: newElementId('a'), to_file: false };
  }
}

/** The allowed Command forms, in the picker's order. Tested against
 *  `src-tauri/tests/fixtures/automation_command_forms.json`. */
export const COMMAND_FORMS: readonly { word: string; label: string; arg: 'none' | 'seconds' | 'duration' | 'text' | 'required_text'; own_timed_only: boolean }[] = [
  { word: 'slow', label: 'Slow mode on', arg: 'seconds', own_timed_only: false },
  { word: 'slowoff', label: 'Slow mode off', arg: 'none', own_timed_only: false },
  { word: 'followers', label: 'Followers-only on', arg: 'duration', own_timed_only: false },
  { word: 'followersoff', label: 'Followers-only off', arg: 'none', own_timed_only: false },
  { word: 'subscribers', label: 'Subscribers-only on', arg: 'none', own_timed_only: false },
  { word: 'subscribersoff', label: 'Subscribers-only off', arg: 'none', own_timed_only: false },
  { word: 'uniquechat', label: 'Unique chat on', arg: 'none', own_timed_only: false },
  { word: 'uniquechatoff', label: 'Unique chat off', arg: 'none', own_timed_only: false },
  { word: 'emoteonly', label: 'Emote-only on', arg: 'none', own_timed_only: false },
  { word: 'emoteonlyoff', label: 'Emote-only off', arg: 'none', own_timed_only: false },
  { word: 'marker', label: 'Add a stream marker', arg: 'text', own_timed_only: false },
  { word: 'settitle', label: 'Set the stream title', arg: 'required_text', own_timed_only: true },
  { word: 'setgame', label: 'Set the category', arg: 'required_text', own_timed_only: true },
];

/** The word and argument of a stored command line: `/slow 60` is ['slow', '60']. */
export function splitCommand(line: string): [string, string] {
  const body = line.trim().replace(/^\//, '');
  const i = body.search(/\s/);
  return i < 0 ? [body.toLowerCase(), ''] : [body.slice(0, i).toLowerCase(), body.slice(i).trim()];
}

export const joinCommand = (word: string, arg: string): string => (arg.trim() ? `/${word} ${arg.trim()}` : `/${word}`);

/** What a command's argument field shows: what was typed for this command word (a trailing space included, so
 *  "Just Chatting" can be typed), else the argument stored in the line. */
export const shownArgument = (typed: { word: string; text: string } | null, word: string, stored: string): string =>
  typed && typed.word === word ? typed.text : stored;

/** `pick_category`'s rule: the row whose name equals the typed one in any case. */
export function pickCategory(rows: readonly { id?: string; name?: string }[], typed: string): string | null {
  const wanted = typed.trim().toLowerCase();
  const row = rows.find((r) => (r.name ?? '').trim().toLowerCase() === wanted && r.id && r.id !== '0');
  return row?.name ?? null;
}

/** `/setgame` at save: each Command action whose line is `/setgame <name>` (a plain name, no `{variable}`)
 *  is looked up through `search_categories` and stored by the category's own name. A name that is no
 *  category, or a lookup that failed, comes back as a field error on that action's `line` with the two
 *  sentences both platforms use ("No category named X", "Couldn't look up categories: <reason>"), and
 *  the rule is not sent to Rust. Every other action passes through untouched. */
export async function resolveSetGames(
  actions: readonly AutomationAction[],
): Promise<{ actions: AutomationAction[]; problems: AutomationFieldError[] }> {
  const out: AutomationAction[] = [];
  const problems: AutomationFieldError[] = [];
  for (const [i, a] of actions.entries()) {
    const [word, arg] = a.kind === 'command' ? splitCommand(a.line) : ['', ''];
    if (a.kind !== 'command' || word !== 'setgame' || !arg.trim() || arg.includes('{')) {
      out.push(a);
      continue;
    }
    try {
      const rows = await invoke<{ id?: string; name?: string }[]>('search_categories', { query: arg.trim(), limit: 10 });
      const name = pickCategory(rows, arg);
      if (name) {
        out.push({ ...a, line: joinCommand('setgame', name) });
      } else {
        out.push(a);
        problems.push({ path: `actions[${i}].line`, message: `No category named ${arg.trim()}` });
      }
    } catch (e) {
      out.push(a);
      problems.push({ path: `actions[${i}].line`, message: `Couldn't look up categories: ${errorText(e)}` });
    }
  }
  return { actions: out, problems };
}

/** Save a rule the way the editor does: `/setgame` lines are looked up first, and on a miss or a failed lookup
 *  nothing is sent (`answer` is null and `problems` says why). `rule` is what was sent. */
export async function saveRuleChecked(
  draft: AutomationRule,
  expectedUpdatedMs: number,
): Promise<{ rule: AutomationRule; answer: AutomationSaveResult | null; problems: AutomationFieldError[] }> {
  const { actions, problems } = await resolveSetGames(draft.actions);
  if (problems.length > 0) return { rule: draft, answer: null, problems };
  const rule = actions.some((a, i) => a !== draft.actions[i]) ? { ...draft, actions } : draft;
  return { rule, answer: await automationsApi.saveRule(rule, expectedUpdatedMs), problems: [] };
}

export type P2ActionKind = 'command' | 'whisper' | 'timeout' | 'delete' | 'pin' | 'announce' | 'shoutout' | 'join_chat' | 'leave_chat' | 'wait' | 'log';
export type P2Action = Extract<AutomationAction, { kind: P2ActionKind }>;

export const isP2Action = (a: AutomationAction): a is P2Action =>
  !(a.kind === 'reply' || a.kind === 'send' || a.kind === 'notify');

/** Chat-message triggers: the moderation actions work on one. */
export const isChatMessageTrigger = (t: AutomationTrigger): boolean => t.kind === 'message' || t.kind === 'mention' || t.kind === 'first_message';
export const isTimedTrigger = (t: AutomationTrigger): boolean => t.kind === 'timer' || t.kind === 'clock' || t.kind === 'uptime' || t.kind === 'delay';

/** The P2 actions a rule offers in its Add row (the main account only, as Rust holds it otherwise). */
export function offeredP2Actions(when: AutomationTrigger, sendAsIsMain: boolean): P2ActionKind[] {
  const out: P2ActionKind[] = ['wait', 'log'];
  if (!sendAsIsMain) return out;
  if (when.kind === 'whisper') return ['whisper', ...out];
  const chat = isChatMessageTrigger(when);
  const raid = when.kind === 'channel_event' && when.events.includes('raid');
  const list: P2ActionKind[] = ['command', 'whisper'];
  if (chat) list.push('announce', 'timeout', 'delete', 'pin', 'shoutout');
  else if (raid) list.push('shoutout');
  return [...list, 'join_chat', 'leave_chat', ...out];
}

export function triggerOf(kind: TriggerKind): AutomationTrigger {
  switch (kind) {
    case 'message':
      return { kind, text: { mode: 'contains', pattern: '', case_sensitive: false } };
    case 'mention':
      return { kind, replies_too: true };
    case 'first_message':
      return { kind };
    case 'channel_event':
      return { kind, events: ['sub'], min_months: 0, min_gifts: 0, min_viewers: 0, min_bits: 0 };
    case 'badge_drop':
      return { kind, on: 'both', link: { preset: 'streamnook', custom: '', if_not_ready: 'streamnook' }, batch_text: '' };
    case 'whisper':
      return { kind, text: null };
    case 'timer':
      return { kind, every_seconds: 900, min_messages: 10 };
    case 'clock':
      return { kind, at: '19:00', days: [] };
    case 'uptime':
      return { kind, seconds: 3600, late_ok: false };
    case 'delay':
      return { kind, seconds: 60 };
  }
}

/** The rule with a new trigger. A badge rule runs in your own channel, only
 *  while you're live, with no chat cooldown, and posts rather than replies,
 *  as Rust's check requires; leaving it gives the chat cooldown back. */
export function withTrigger(rule: AutomationRule, kind: TriggerKind): AutomationRule {
  if (rule.when.kind === kind) return rule;
  const when = triggerOf(kind);
  if (kind !== 'badge_drop') {
    const cooldown =
      rule.when.kind === 'badge_drop' && rule.limits.rule_cooldown_seconds === 0
        ? DEFAULT_LIMITS.rule_cooldown_seconds
        : rule.limits.rule_cooldown_seconds;
    const scope = rule.only_if.channels;
    const own = { kind: 'own' } as const;
    const channels = (isTimedTrigger(when) && scope.kind === 'any_open') || when.kind === 'whisper' ? own : scope;
    const only_live = isTimedTrigger(when) && when.kind !== 'delay' ? true : rule.only_if.only_live;
    return { ...rule, when, only_if: { ...rule.only_if, channels, only_live, expression: isChatMessageTrigger(when) || when.kind === 'channel_event' ? rule.only_if.expression : null }, limits: { ...rule.limits, rule_cooldown_seconds: cooldown } };
  }
  return {
    ...rule,
    when,
    only_if: { ...rule.only_if, channels: { kind: 'own' }, only_live: true },
    limits: { ...rule.limits, rule_cooldown_seconds: 0 },
    actions: rule.actions.map(
      (a): AutomationAction => (a.kind === 'reply' ? { kind: 'send', id: a.id, to: null, texts: a.texts, pick: a.pick } : a),
    ),
  };
}

export const accountRef = (a: AutomationAccount): AutomationAccountRef => ({
  provider: 'twitch',
  id: a.id,
  login: a.login,
  name: a.login,
});

/** A new rule's start: answers mentions of the owner with a reply. */
export function blankRule(owner: AutomationAccountRef): AutomationRule {
  return {
    id: '',
    name: '',
    enabled: true,
    live: false,
    owner,
    send_as: owner,
    when: triggerOf('mention'),
    only_if: {
      channels: { kind: 'own' },
      roles: [],
      skip_users: [],
      only_users: [],
      only_live: false,
      only_away: false,
      shared_chat: false,
      hours: null,
      expression: null,
    },
    actions: [actionOf('reply')],
    limits: { ...DEFAULT_LIMITS },
    vary: false,
    origin: null,
    created_ms: 0,
    updated_ms: 0,
  };
}

function withNewIds(a: AutomationAction): AutomationAction {
  if (a.kind === 'join_chat' || a.kind === 'leave_chat') {
    return { ...a, id: newElementId('a'), channel: { ...a.channel, id: newElementId('c') } };
  }
  if (!('texts' in a)) return { ...a, id: newElementId('a') };
  const texts = a.texts.map((l) => ({ ...l, id: newElementId('l') }));
  if (a.kind === 'whisper') return { ...a, id: newElementId('a'), texts };
  if (a.kind === 'send') {
    return { ...a, id: newElementId('a'), texts, to: a.to ? { ...a.to, id: newElementId('c') } : null };
  }
  return { ...a, id: newElementId('a'), texts };
}

/** A copy to save as a new rule: new ids, in Test, switched on. */
export function duplicateRule(rule: AutomationRule): AutomationRule {
  const copy = structuredClone(rule);
  const scope = copy.only_if.channels;
  return {
    ...copy,
    id: '',
    name: `${rule.name} copy`.slice(0, 60).trim(),
    enabled: true,
    live: false,
    origin: null,
    created_ms: 0,
    updated_ms: 0,
    only_if: {
      ...copy.only_if,
      channels:
        scope.kind === 'these' ? { kind: 'these', list: scope.list.map((c) => ({ ...c, id: newElementId('c') })) } : scope,
    },
    actions: copy.actions.map(withNewIds),
  };
}

/** Whether two versions of a rule differ in what the editor changes (the
 *  switch, Test or Live and the stamps are set elsewhere). */
export function sameRuleBody(a: AutomationRule, b: AutomationRule): boolean {
  const body = (r: AutomationRule) => JSON.stringify({ ...r, enabled: true, live: false, created_ms: 0, updated_ms: 0 });
  return body(a) === body(b);
}

/** Names typed as one line: split on commas and spaces, `@` dropped, lowercased, once each. */
export function splitNames(text: string): string[] {
  const names = text
    .split(/[\s,]+/)
    .map((s) => s.trim().replace(/^@/, '').toLowerCase())
    .filter(Boolean);
  return [...new Set(names)];
}

export const isOutward = (a: AutomationAction): boolean =>
  !(a.kind === 'notify' || a.kind === 'log' || a.kind === 'join_chat' || a.kind === 'leave_chat' || a.kind === 'wait');

/** Where a rule lands when it moves one place: before the returned id, or at
 *  the end for null. Null overall when it is already at that end. */
export function moveBeforeId(ids: readonly string[], index: number, dir: -1 | 1): { beforeId: string | null } | null {
  const target = index + dir;
  if (index < 0 || index >= ids.length || target < 0 || target >= ids.length) return null;
  if (dir === -1) return { beforeId: ids[target] };
  return { beforeId: index + 2 < ids.length ? ids[index + 2] : null };
}

/** A sample that makes the draft act, for the editor's live preview. */
export function previewSample(draft: AutomationRule): AutomationTestSample {
  const scope = draft.only_if.channels;
  const channel = scope.kind === 'these' && scope.list[0] ? scope.list[0].name : draft.owner.login;
  const base: AutomationTestSample = {
    text: 'Hello there',
    channel,
    sender: draft.only_if.only_users[0] ?? 'viewer',
    roles: draft.only_if.roles.slice(0, 1),
    first_message: false,
    event: null,
    badge_feed_id: null,
    draft,
  };
  const when = draft.when;
  switch (when.kind) {
    case 'message':
      return { ...base, text: when.text.pattern.trim() || base.text };
    case 'mention':
      return { ...base, text: `@${draft.owner.login} are you around?` };
    case 'first_message':
      return { ...base, first_message: true, text: 'Hi everyone!' };
    case 'channel_event':
      return {
        ...base,
        text: '',
        event: {
          kind: when.events[0] ?? 'sub',
          months: Math.max(when.min_months, 12),
          gifts: Math.max(when.min_gifts, 5),
          viewers: Math.max(when.min_viewers, 120),
          bits: Math.max(when.min_bits, 500),
        },
      };
    case 'badge_drop':
    case 'timer':
    case 'clock':
    case 'uptime':
    case 'delay':
      return { ...base, text: '' };
    case 'whisper':
      return { ...base, text: when.text?.pattern.trim() || 'Hello' };
  }
}

/** Try it's event: the one number that event carries. */
export function eventSample(kind: AutomationEventKind, amount: number): AutomationTestEvent {
  const n = Math.max(0, Math.round(amount));
  return {
    kind,
    months: kind === 'resub' ? n : 0,
    gifts: kind === 'gift' || kind === 'gift_bomb' ? n : 0,
    viewers: kind === 'raid' ? n : 0,
    bits: kind === 'cheer' ? n : 0,
  };
}

/** The name of an event's number, or null for an event without one. */
export function amountLabel(kind: AutomationEventKind): string | null {
  switch (kind) {
    case 'resub':
      return 'Months';
    case 'gift':
    case 'gift_bomb':
      return 'Gifts';
    case 'raid':
      return 'Viewers';
    case 'cheer':
      return 'Bits';
    default:
      return null;
  }
}

// ---------- Words for times -------------------------------------------------

/** "just now", "3 min ago", "2 h ago", "yesterday", "4 days ago". */
export function agoText(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  return d === 1 ? 'yesterday' : `${d} days ago`;
}

/** How long since: "less than a minute", "12 minutes", "2 hours". */
export function spanText(fromMs: number, now: number): string {
  const m = Math.max(0, Math.floor((now - fromMs) / 60_000));
  if (m < 1) return 'less than a minute';
  if (m < 60) return m === 1 ? '1 minute' : `${m} minutes`;
  const h = Math.floor(m / 60);
  return h === 1 ? '1 hour' : `${h} hours`;
}

/** A gap in seconds, short: "30 s", "1.5 min", "10 min". */
export function secondsText(s: number): string {
  if (s < 60) return `${s} s`;
  const m = s / 60;
  return `${Number.isInteger(m) ? m : m.toFixed(1)} min`;
}

/** A rule's record: "Hasn't acted yet", "Acted once, last 2 min ago". */
export function actedText(count: number, lastMs: number | null, now: number): string {
  if (count <= 0 || lastMs === null) return "Hasn't acted yet";
  const times = count === 1 ? 'once' : `${count} times`;
  return `Acted ${times}, last ${agoText(lastMs, now)}`;
}

export const clockText = (ms: number): string =>
  new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

export const dayText = (ms: number): string =>
  new Date(ms).toLocaleDateString([], { month: 'short', day: 'numeric' });
