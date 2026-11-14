// The P2 editor surfaces: the whisper, timer, clock, uptime and delay
// triggers, the expression, the Add row's P2 actions and each P2 action's
// fields. Rust checks every field; this only asks and shows its errors.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Minus, Plus } from 'lucide-react';
import type {
  AutomationAction,
  AutomationFieldError,
  AutomationRule,
  AutomationTrigger,
  AutomationWeekday,
  AutomationWhisperTarget,
} from '../../../types';
import { SwitchList } from '../_primitives';
import { IS_MOBILE } from '../../../utils/platform';
import {
  COMMAND_FORMS,
  actionOf,
  errorAt,
  errorText,
  joinCommand,
  nextTimeout,
  offeredP2Actions,
  shownArgument,
  splitCommand,
  timeoutWords,
  type P2Action,
} from '../../../utils/automations';
import { ChannelPicker } from '../ChannelPicker';
import { ACTION_NAMES, ANNOUNCE_COLORS } from './labels';
import { Field, HINT, INPUT, NumberField, QUIET_BUTTON, SELECT, SwitchLine } from './parts';

/** A stepper end inside the joined timeout control. */
const STEP_BUTTON = `inline-flex items-center justify-center text-textSecondary transition-colors hover:text-textPrimary ${
  IS_MOBILE ? 'p-2.5' : 'h-8 w-8 p-0'
}`;

const DAYS: { id: AutomationWeekday; label: string }[] = [
  { id: 'mon', label: 'Monday' },
  { id: 'tue', label: 'Tuesday' },
  { id: 'wed', label: 'Wednesday' },
  { id: 'thu', label: 'Thursday' },
  { id: 'fri', label: 'Friday' },
  { id: 'sat', label: 'Saturday' },
  { id: 'sun', label: 'Sunday' },
];

type P2Trigger = Extract<AutomationTrigger, { kind: 'whisper' | 'timer' | 'clock' | 'uptime' | 'delay' }>;

export const isP2Trigger = (t: AutomationTrigger): t is P2Trigger =>
  t.kind === 'whisper' || t.kind === 'timer' || t.kind === 'clock' || t.kind === 'uptime' || t.kind === 'delay';

/** The When fields of a whisper or timed trigger. */
export function P2TriggerFields({
  when,
  errors,
  onChange,
}: {
  when: P2Trigger;
  errors: readonly AutomationFieldError[];
  onChange: (next: AutomationTrigger) => void;
}) {
  switch (when.kind) {
    case 'whisper':
      return (
        <>
          <Field label="Words" hint="Leave empty to answer any whisper from someone you have whispered with." error={errorAt(errors, 'when.text.pattern')}>
            <input
              value={when.text?.pattern ?? ''}
              maxLength={200}
              aria-label="Words"
              spellCheck={false}
              onChange={(e) =>
                onChange({
                  ...when,
                  text: e.target.value === '' ? null : { mode: when.text?.mode ?? 'contains', pattern: e.target.value, case_sensitive: false },
                })
              }
              className={INPUT}
            />
          </Field>
          <p className={HINT}>Only whispers from people you have whispered with before. It runs while your main account is the one signed in here.</p>
        </>
      );
    case 'timer':
      return (
        <>
          <Field label="Every" error={errorAt(errors, 'when.every_seconds')}>
            <NumberField
              value={Math.round(when.every_seconds / 60)}
              min={5}
              max={720}
              label="Minutes"
              suffix="minutes"
              onChange={(v) => onChange({ ...when, every_seconds: v * 60 })}
            />
          </Field>
          <Field label="Only after" hint="Skipped until this many other people's messages came since it last ran." error={errorAt(errors, 'when.min_messages')}>
            <NumberField
              value={when.min_messages}
              min={0}
              max={1000}
              label="Messages"
              suffix="messages"
              onChange={(v) => onChange({ ...when, min_messages: v })}
            />
          </Field>
        </>
      );
    case 'clock':
      return (
        <>
          <Field label="At" error={errorAt(errors, 'when.at')}>
            <input type="time" value={when.at} aria-label="At" onChange={(e) => onChange({ ...when, at: e.target.value })} className={INPUT} />
          </Field>
          <Field label="Days" hint="Leave them all off for every day.">
            <SwitchList
              items={DAYS}
              isOn={(id) => when.days.includes(id as AutomationWeekday)}
              onToggle={(id) => {
                const day = id as AutomationWeekday;
                onChange({ ...when, days: when.days.includes(day) ? when.days.filter((d) => d !== day) : [...when.days, day] });
              }}
            />
          </Field>
        </>
      );
    case 'uptime':
      return (
        <>
          <Field label="Stream reaches" error={errorAt(errors, 'when.seconds')}>
            <NumberField
              value={Math.round(when.seconds / 60)}
              min={1}
              max={2880}
              label="Minutes"
              suffix="minutes"
              onChange={(v) => onChange({ ...when, seconds: v * 60 })}
            />
          </Field>
          <p className={HINT}>Runs once per broadcast, within 2 minutes of that moment.</p>
        </>
      );
    case 'delay':
      return (
        <Field label="After the chat opens" error={errorAt(errors, 'when.seconds')}>
          <NumberField
            value={when.seconds}
            min={1}
            max={172800}
            label="Seconds"
            suffix="seconds"
            onChange={(v) => onChange({ ...when, seconds: v })}
          />
        </Field>
      );
  }
}

/** The saved-filter expression, a plain text field with Rust's errors. */
export function ExpressionField({
  value,
  errors,
  onChange,
}: {
  value: string | null;
  errors: readonly AutomationFieldError[];
  onChange: (next: string | null) => void;
}) {
  return (
    <Field
      label="Expression"
      hint="The same language as your saved chat filters, for example author.subbed && message.length > 20."
      error={errorAt(errors, 'only_if.expression')}
    >
      <input
        value={value ?? ''}
        maxLength={500}
        aria-label="Expression"
        spellCheck={false}
        onChange={(e) => onChange(e.target.value === '' ? null : e.target.value)}
        className={INPUT}
      />
    </Field>
  );
}

/** The P2 buttons of the Add row. */
export function P2AddActions({
  rule,
  sendAsIsMain,
  onAdd,
}: {
  rule: AutomationRule;
  sendAsIsMain: boolean;
  onAdd: (action: AutomationAction) => void;
}) {
  return (
    <>
      {offeredP2Actions(rule.when, sendAsIsMain).map((kind) => (
        <button key={kind} type="button" onClick={() => onAdd(actionOf(kind))} className={QUIET_BUTTON}>
          <Plus size={13} />
          {ACTION_NAMES[kind]}
        </button>
      ))}
    </>
  );
}

interface Person {
  user_id: string;
  login: string;
  name: string;
}

/** People you have whispered with, from Rust (`knows`). */
function usePeople(): Person[] | null {
  const [people, setPeople] = useState<Person[] | null>(null);
  useEffect(() => {
    let live = true;
    invoke<Person[]>('automations_whisper_people')
      .then((rows) => live && setPeople(rows))
      .catch(() => live && setPeople([]));
    return () => {
      live = false;
    };
  }, []);
  return people;
}

function CommandFields({
  action,
  at,
  errors,
  timedOwn,
  onChange,
}: {
  action: Extract<P2Action, { kind: 'command' }>;
  at: string;
  errors: readonly AutomationFieldError[];
  timedOwn: boolean;
  onChange: (next: AutomationAction) => void;
}) {
  const [word, arg] = splitCommand(action.line);
  const form = COMMAND_FORMS.find((f) => f.word === word) ?? COMMAND_FORMS[0];
  const forms = COMMAND_FORMS.filter((f) => timedOwn || !f.own_timed_only);
  // The stored line is trimmed, so the field keeps what was typed for this command word itself.
  const [typed, setTyped] = useState<{ word: string; text: string } | null>(null);
  const set = (w: string, a: string) => onChange({ ...action, line: joinCommand(w, a) });

  // /setgame is looked up when the rule is saved (RuleEditor's `save` runs `resolveSetGames`), never on blur: a
  // late answer could put a name back over a newer edit. Its two failures arrive as this field's error.

  return (
    <div className="space-y-3">
      <Field label="Command" error={form.arg === 'none' ? errorAt(errors, `${at}.line`) : undefined}>
        <select value={form.word} aria-label="Command" onChange={(e) => {
            setTyped(null);
            set(e.target.value, '');
          }}
          className={SELECT}
        >
          {forms.map((f) => (
            <option key={f.word} value={f.word}>
              {f.label}
            </option>
          ))}
        </select>
      </Field>
      {form.arg !== 'none' && (
        <Field
          label={form.arg === 'seconds' ? 'Seconds' : form.arg === 'duration' ? 'Duration' : form.word === 'setgame' ? 'Category' : form.word === 'settitle' ? 'Title' : 'Description'}
          hint={form.arg === 'duration' ? 'Like 10m, 2h or 1d. Empty lets anyone who follows talk.' : form.arg === 'seconds' ? '3 to 120. Empty is 30.' : undefined}
          error={errorAt(errors, `${at}.line`)}
        >
          <input
            value={shownArgument(typed, form.word, arg)}
            aria-label="Argument"
            maxLength={form.word === 'settitle' || form.word === 'marker' ? 140 : 100}
            onChange={(e) => {
              setTyped({ word: form.word, text: e.target.value });
              set(form.word, e.target.value);
            }}
            className={INPUT}
          />
        </Field>
      )}
      {form.own_timed_only && <p className={HINT}>Works on a timer or a clock in your own channel, acting as the channel's owner.</p>}
    </div>
  );
}

/** A P2 action's fields (the header, move and remove are the ActionEditor's). */
export function P2ActionFields({
  action,
  at,
  errors,
  timedOwn,
  onChange,
}: {
  action: P2Action;
  at: string;
  errors: readonly AutomationFieldError[];
  timedOwn: boolean;
  onChange: (next: AutomationAction) => void;
}) {
  const people = usePeople();
  switch (action.kind) {
    case 'command':
      return <CommandFields action={action} at={at} errors={errors} timedOwn={timedOwn} onChange={onChange} />;
    case 'whisper': {
      const to = action.to;
      return (
        <div className="space-y-3">
          <Field
            label="To"
            hint={`People you have whispered with. Leave it on the sender to answer a whisper.${people !== null && people.length === 0 ? ' No one yet. Whisper someone once yourself and they show up here.' : ''}`}
            error={errorAt(errors, `${at}.to`)}>
            <select
              value={to?.user_id ?? ''}
              aria-label="To"
              onChange={(e) => {
                const p = people?.find((x) => x.user_id === e.target.value);
                const target: AutomationWhisperTarget | null = p ? { user_id: p.user_id, login: p.login, name: p.name } : null;
                onChange({ ...action, to: target });
              }}
              className={SELECT}
            >
              <option value="">The sender</option>
              {(people ?? []).map((p) => (
                <option key={p.user_id} value={p.user_id}>
                  {p.name || p.login}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Message" error={errorAt(errors, `${at}.texts`) ?? errorAt(errors, `${at}.texts[0].text`)}>
            <textarea
              value={action.texts[0]?.text ?? ''}
              rows={2}
              maxLength={500}
              aria-label="Message"
              onChange={(e) =>
                onChange({ ...action, texts: [{ id: action.texts[0]?.id ?? '', text: e.target.value }, ...action.texts.slice(1)] })
              }
              className={`${INPUT} resize-y`}
            />
          </Field>
        </div>
      );
    }
    case 'timeout':
      return (
        <div className="space-y-3">
          <Field label="For" hint="1 second to 14 days. Never a permanent ban." error={errorAt(errors, `${at}.seconds`)}>
            <div className="glass-input inline-flex h-8 items-center">
              <button
                type="button"
                aria-label="Shorter"
                className={STEP_BUTTON}
                onClick={() => onChange({ ...action, seconds: Math.max(1, nextTimeout(action.seconds, -1)) })}
              >
                <Minus size={13} />
              </button>
              <span className="min-w-[8ch] text-center text-[13px] tabular-nums text-textPrimary">{timeoutWords(action.seconds)}</span>
              <button
                type="button"
                aria-label="Longer"
                className={STEP_BUTTON}
                onClick={() => onChange({ ...action, seconds: Math.min(1_209_600, nextTimeout(action.seconds, 1)) })}
              >
                <Plus size={13} />
              </button>
            </div>
          </Field>
          <Field label="Reason" error={errorAt(errors, `${at}.reason`)}>
            <input
              value={action.reason}
              maxLength={500}
              aria-label="Reason"
              onChange={(e) => onChange({ ...action, reason: e.target.value })}
              className={INPUT}
            />
          </Field>
        </div>
      );
    case 'delete':
    case 'shoutout':
      return <p className={HINT}>{action.kind === 'delete' ? "Deletes the message that started this rule." : 'Shouts out the person who started this rule (a raid: the raider).'}</p>;
    case 'pin':
      return (
        <Field label="For" hint="Leave it off to keep it pinned until the stream ends." error={errorAt(errors, `${at}.seconds`)}>
          <div className="space-y-2">
            <SwitchLine
              label="Unpin after a while"
              on={action.seconds !== null}
              onChange={() => onChange({ ...action, seconds: action.seconds === null ? 300 : null })}
            />
            {action.seconds !== null && (
              <NumberField
                value={Math.round(action.seconds / 30) * 30}
                min={30}
                max={1800}
                label="Seconds"
                suffix="seconds"
                onChange={(v) => onChange({ ...action, seconds: v })}
              />
            )}
          </div>
        </Field>
      );
    case 'announce':
      return (
        <div className="space-y-3">
          <Field label="Announcement" error={errorAt(errors, `${at}.text`)}>
            <textarea
              value={action.text}
              rows={2}
              maxLength={500}
              aria-label="Announcement"
              onChange={(e) => onChange({ ...action, text: e.target.value })}
              className={`${INPUT} resize-y`}
            />
          </Field>
          <Field label="Color" error={errorAt(errors, `${at}.color`)}>
            <select
              value={action.color}
              aria-label="Color"
              onChange={(e) => onChange({ ...action, color: e.target.value as typeof action.color })}
              className={SELECT}
            >
              {ANNOUNCE_COLORS.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
      );
    case 'join_chat':
    case 'leave_chat':
      return (
        <Field label="Chat" error={errorAt(errors, `${at}.channel`)}>
          <ChannelPicker
            login={action.channel.name || undefined}
            label="Choose the chat"
            onPick={(login, id) => onChange({ ...action, channel: { ...action.channel, key: '', room_id: id, name: login } })}
          />
        </Field>
      );
    case 'wait':
      return (
        <Field label="Wait" error={errorAt(errors, `${at}.seconds`)}>
          <NumberField value={action.seconds} min={1} max={300} label="Seconds" suffix="seconds" onChange={(v) => onChange({ ...action, seconds: v })} />
        </Field>
      );
    case 'log':
      return (
        <div className="space-y-2">
          <SwitchLine
            label="Also write to a file"
            description="One file a day, up to 5 MB, kept for 14 days. It is never part of the support zip."
            on={action.to_file}
            onChange={() => onChange({ ...action, to_file: !action.to_file })}
          />
          {action.to_file && <LogFolderButton />}
        </div>
      );
  }
}

/** Open folder on the desktop, Share on a phone. */
function LogFolderButton() {
  const [problem, setProblem] = useState<string | null>(null);
  const run = () => {
    const command = IS_MOBILE ? 'automations_share_log' : 'automations_open_log_folder';
    invoke(command).catch((e) => setProblem(errorText(e)));
  };
  return (
    <div className="space-y-1">
      <button type="button" onClick={run} className={QUIET_BUTTON}>
        {IS_MOBILE ? 'Share log' : 'Open folder'}
      </button>
      {problem && <p className="text-[12px] text-error">{problem}</p>}
    </div>
  );
}
