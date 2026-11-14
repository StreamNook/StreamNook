// Try it: a sample message, chat, sender, roles, event or recent badge drop,
// run against every rule. Nothing is sent, counted or claimed.
import { useEffect, useState } from 'react';
import { Check, FlaskConical } from 'lucide-react';
import type { AutomationEventKind, AutomationSenderRole } from '../../../types';
import { SwitchList } from '../_primitives';
import {
  amountLabel,
  automationsApi,
  errorText,
  eventSample,
  type AutomationBadgeSample,
  type AutomationStatus,
  type AutomationTestResult,
} from '../../../utils/automations';
import { ACTION_WORDS, ROLE_ITEMS } from './labels';
import { Field, HINT, INPUT, NumberField, PRIMARY_BUTTON, SELECT, SwitchLine } from './parts';

const EVENT_CHOICES: { value: AutomationEventKind; label: string; amount: number }[] = [
  { value: 'sub', label: 'A new sub', amount: 0 },
  { value: 'resub', label: 'A resub', amount: 12 },
  { value: 'gift', label: 'A gifted sub', amount: 1 },
  { value: 'gift_bomb', label: 'A gift bomb', amount: 5 },
  { value: 'raid', label: 'A raid', amount: 50 },
  { value: 'cheer', label: 'A cheer', amount: 100 },
  { value: 'announcement', label: 'An announcement', amount: 0 },
];

export default function TryIt({ status }: { status: AutomationStatus | null }) {
  const mainLogin = status?.accounts.find((a) => a.main)?.login ?? '';
  const [text, setText] = useState('');
  const [channel, setChannel] = useState('');
  const [sender, setSender] = useState('');
  const [roles, setRoles] = useState<AutomationSenderRole[]>([]);
  const [first, setFirst] = useState(false);
  const [eventKind, setEventKind] = useState<AutomationEventKind | ''>('');
  const [amount, setAmount] = useState(0);
  const [badgeId, setBadgeId] = useState('');
  const [samples, setSamples] = useState<AutomationBadgeSample[]>([]);
  const [result, setResult] = useState<AutomationTestResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    automationsApi
      .badgeSamples()
      .then((s) => {
        if (live) setSamples(s);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const run = () => {
    setBusy(true);
    setProblem(null);
    automationsApi
      .test({
        text,
        channel: channel.trim().replace(/^#/, '').toLowerCase() || mainLogin,
        sender: sender.trim().replace(/^@/, '').toLowerCase() || 'viewer',
        roles,
        first_message: first,
        event: eventKind ? eventSample(eventKind, amount) : null,
        badge_feed_id: badgeId || null,
        draft: null,
      })
      .then(setResult)
      .catch((e: unknown) => setProblem(errorText(e)))
      .finally(() => setBusy(false));
  };

  const label = eventKind ? amountLabel(eventKind) : null;

  return (
    <div className="settings-card space-y-4 rounded-lg p-4">
      <Field label="Message">
        <textarea
          value={text}
          rows={2}
          aria-label="Message"
          onChange={(e) => setText(e.target.value)}
          placeholder={mainLogin ? `@${mainLogin} are you there?` : 'Hello there'}
          className={`${INPUT} resize-y`}
        />
      </Field>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Chat">
          <input
            value={channel}
            aria-label="Chat"
            spellCheck={false}
            onChange={(e) => setChannel(e.target.value)}
            placeholder={mainLogin || 'channel'}
            className={INPUT}
          />
        </Field>
        <Field label="Sent by">
          <input
            value={sender}
            aria-label="Sent by"
            spellCheck={false}
            onChange={(e) => setSender(e.target.value)}
            placeholder="viewer"
            className={INPUT}
          />
        </Field>
      </div>
      <Field label="Their roles" hint="Leave them all off for an ordinary viewer.">
        <SwitchList
          items={ROLE_ITEMS}
          isOn={(id) => roles.some((r) => r === id)}
          onToggle={(id) => {
            const role = ROLE_ITEMS.find((item) => item.id === id)?.id;
            if (!role) return;
            setRoles((list) => (list.includes(role) ? list.filter((r) => r !== role) : [...list, role]));
          }}
        />
      </Field>
      <SwitchLine label="First message here" description="As if they never chatted here before." on={first} onChange={() => setFirst((v) => !v)} />
      <div className="flex flex-wrap items-end gap-3">
        <Field label="Event">
          <select
            value={eventKind}
            aria-label="Event"
            onChange={(e) => {
              const choice = EVENT_CHOICES.find((c) => c.value === e.target.value);
              setEventKind(choice?.value ?? '');
              setAmount(choice?.amount ?? 0);
            }}
            className={SELECT}
          >
            <option value="">None</option>
            {EVENT_CHOICES.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </Field>
        {label && (
          <Field label={label}>
            <NumberField value={amount} min={0} max={1_000_000} label={label} onChange={setAmount} />
          </Field>
        )}
      </div>
      {samples.length > 0 && (
        <Field label="Try with" hint="A recent badge drop, for badge rules.">
          <select value={badgeId} aria-label="Try with" onChange={(e) => setBadgeId(e.target.value)} className={SELECT}>
            <option value="">No badge</option>
            {samples.map((s) => (
              <option key={`${s.feed_id}-${s.phase}`} value={s.feed_id}>
                {s.name} ({s.phase === 'earnable' ? 'can be earned' : 'announced'})
              </option>
            ))}
          </select>
        </Field>
      )}
      <div className="flex items-center gap-3">
        <button type="button" onClick={run} disabled={busy} className={PRIMARY_BUTTON}>
          <FlaskConical size={14} />
          {busy ? 'Trying' : 'Try it'}
        </button>
        <span className={HINT}>Nothing is sent.</span>
      </div>
      {problem && (
        <p role="alert" className="text-[12px] text-error">
          {problem}
        </p>
      )}
      {result && (
        <div className="space-y-2 border-t border-borderSubtle/60 pt-3">
          {result.rules.length === 0 && <p className={HINT}>You have no rules to try yet.</p>}
          {result.rules.map((r) => (
            <div key={r.rule_id} className="rounded-md bg-background/40 px-3 py-2.5">
              <div className="flex items-center justify-between gap-3">
                <span className="truncate text-[13px] font-medium text-textPrimary">{r.name}</span>
                <span
                  className={`inline-flex shrink-0 items-center gap-1 text-[12px] ${r.fired ? 'font-medium text-textPrimary' : 'text-textMuted'}`}
                >
                  {r.fired && <Check size={13} className="text-accent" />}
                  {r.fired ? 'Would act' : "Wouldn't act"}
                </span>
              </div>
              {r.fired &&
                r.texts.map((t) => (
                  <p key={t.action_id} className="mt-1 text-[12px] leading-relaxed text-textSecondary">
                    <span className="mr-1.5 text-textMuted">{ACTION_WORDS[t.kind]}</span>
                    {t.text}
                  </p>
                ))}
              {r.trace.length > 0 && (
                <ul className="mt-1.5 space-y-0.5 text-[11.5px] leading-relaxed text-textMuted">
                  {r.trace.map((line, i) => (
                    <li key={i}>{line}</li>
                  ))}
                </ul>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
