// One rule, opened for editing: its name, When, If, Do and Send as, a live
// preview of what it would post, and Test or Live, Duplicate and Delete. The
// draft is the editor's own until Save; Rust checks it, merges it over the
// stored rule and refuses a save made from an older copy.
import { useEffect, useState } from 'react';
import { Copy, Plus, Trash2, X } from 'lucide-react';
import type {
  AutomationAction,
  AutomationChannelScope,
  AutomationConditions,
  AutomationFieldError,
  AutomationLimits,
  AutomationMatchMode,
  AutomationPlatform,
  AutomationRule,
  AutomationSenderRole,
  AutomationTrigger,
  AutomationWeekday,
} from '../../../types';
import { SegmentedSelect, SwitchList } from '../_primitives';
import { ChannelPicker } from '../ChannelPicker';
import { IS_MOBILE } from '../../../utils/platform';
import { ProviderMark } from '../../ProviderLogo';
import { makeKey } from '../../../utils/providerKey';
import { refreshAutomationStatus } from '../../../hooks/useAutomationStatus';
import {
  accountRef,
  actionOf,
  automationsApi,
  canPost,
  errorAt,
  errorText,
  eventsOn,
  isOutward,
  loadAutomationVariables,
  newElementId,
  platformAccountRef,
  previewSample,
  providerOfKey,
  saveRuleChecked,
  rulePlatform,
  sameRuleBody,
  isEdgeKind,
  isOwnEventKind,
  withPlatforms,
  withTrigger,
  type AutomationAccount,
  type AutomationPlatformAccount,
  type AutomationTestRule,
  type AutomationVariable,
  type TriggerKind,
} from '../../../utils/automations';
import { ActionEditor } from './ActionEditor';
import { EdgeIf, LiveChangeFields, OwnEventFields } from './OwnEventFields';
import { BadgeDropFields } from './BadgeDropFields';
import { WhatChanges } from './WhatChanges';
import { EVENT_ITEMS, P2_TRIGGERS, PLATFORM_ITEMS, ROLE_ITEMS } from './labels';
import { ExpressionField, P2AddActions, P2TriggerFields, isP2Trigger } from './P2Fields';
import {
  Block,
  DANGER_BUTTON,
  Disclosure,
  Field,
  HINT,
  INPUT,
  NamesInput,
  NumberField,
  PRIMARY_BUTTON,
  QUIET_BUTTON,
  SELECT,
  SwitchLine,
} from './parts';

const MAX_ACTIONS = 5;
const MAX_CHATS = 25;
/** How long typing rests before the preview asks Rust again. */
const PREVIEW_DELAY_MS = 400;

const TRIGGERS: { value: TriggerKind; label: string }[] = [
  { value: 'mention', label: 'Someone mentions you' },
  { value: 'message', label: 'Someone says certain words' },
  { value: 'first_message', label: 'Someone chats for the first time' },
  { value: 'channel_event', label: 'Someone subs, gifts, raids or cheers' },
  { value: 'badge_drop', label: 'A new Twitch badge drops' },
  { value: 'follow', label: 'Someone follows you' },
  { value: 'reward_redeemed', label: 'Someone redeems a reward' },
  { value: 'poll_started', label: 'You start a poll' },
  { value: 'prediction_started', label: 'You start a prediction' },
  { value: 'live_change', label: 'A channel goes live or offline' },
  ...P2_TRIGGERS,
];

const MODES: { value: AutomationMatchMode; label: string }[] = [
  { value: 'contains', label: 'Contains this' },
  { value: 'any_word', label: 'Any of these words' },
  { value: 'all_words', label: 'All of these words' },
  { value: 'phrase', label: 'This phrase' },
  { value: 'starts_with', label: 'Starts with this' },
  { value: 'exact', label: 'The whole message' },
  { value: 'regex', label: 'Regular expression' },
];

const SCOPES: { value: AutomationChannelScope['kind']; label: string }[] = [
  { value: 'own', label: 'Your channel' },
  { value: 'these', label: 'These channels' },
  { value: 'any_open', label: 'Any chat you have open' },
  { value: 'watching', label: "The stream you're watching" },
];

const DAYS: { id: AutomationWeekday; label: string }[] = [
  { id: 'mon', label: 'Monday' },
  { id: 'tue', label: 'Tuesday' },
  { id: 'wed', label: 'Wednesday' },
  { id: 'thu', label: 'Thursday' },
  { id: 'fri', label: 'Friday' },
  { id: 'sat', label: 'Saturday' },
  { id: 'sun', label: 'Sunday' },
];

type EventTrigger = Extract<AutomationTrigger, { kind: 'channel_event' }>;
type MinimumField = 'min_months' | 'min_gifts' | 'min_viewers' | 'min_bits';

/** The event trigger with one of its minimums changed. */
function withMinimum(when: EventTrigger, field: MinimumField, value: number): EventTrigger {
  return {
    ...when,
    min_months: field === 'min_months' ? value : when.min_months,
    min_gifts: field === 'min_gifts' ? value : when.min_gifts,
    min_viewers: field === 'min_viewers' ? value : when.min_viewers,
    min_bits: field === 'min_bits' ? value : when.min_bits,
  };
}

const MINIMUMS: { event: 'resub' | 'gift_bomb' | 'raid' | 'cheer'; field: MinimumField; label: string; suffix: string }[] = [
  { event: 'resub', field: 'min_months', label: 'Resub months', suffix: 'months or more' },
  { event: 'gift_bomb', field: 'min_gifts', label: 'Gift bomb size', suffix: 'gifts or more' },
  { event: 'raid', field: 'min_viewers', label: 'Raid size', suffix: 'viewers or more' },
  { event: 'cheer', field: 'min_bits', label: 'Cheer size', suffix: 'bits or more' },
];

function scopeOf(kind: AutomationChannelScope['kind'], current: AutomationChannelScope): AutomationChannelScope {
  switch (kind) {
    case 'these':
      return { kind, list: current.kind === 'these' ? current.list : [] };
    case 'own':
      return { kind };
    case 'any_open':
      return { kind };
    case 'watching':
      return { kind };
  }
}

/** The variable table, once loaded. */
function useVariables(): AutomationVariable[] {
  const [vars, setVars] = useState<AutomationVariable[]>([]);
  useEffect(() => {
    let live = true;
    void loadAutomationVariables().then((v) => {
      if (live) setVars(v);
    });
    return () => {
      live = false;
    };
  }, []);
  return vars;
}

/** What the draft would post, asked again once typing rests. */
function usePreview(draft: AutomationRule): AutomationTestRule | null {
  const [result, setResult] = useState<AutomationTestRule | null>(null);
  const key = JSON.stringify(draft);
  useEffect(() => {
    let live = true;
    const timer = window.setTimeout(() => {
      const rule = JSON.parse(key) as AutomationRule;
      automationsApi
        .test(previewSample(rule))
        .then((r) => {
          if (live) setResult(r.rules[0] ?? null);
        })
        .catch(() => {
          if (live) setResult(null);
        });
    }, PREVIEW_DELAY_MS);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [key]);
  return result;
}

interface RuleEditorProps {
  /** The rule as stored, or a new rule's start. */
  initial: AutomationRule;
  isNew: boolean;
  accounts: readonly AutomationAccount[];
  /** The connected Kick account, for a Kick rule's Send as. */
  platformAccounts: readonly AutomationPlatformAccount[];
  /** Whether rules may post in chats that aren't yours. */
  otherChannelsOn: boolean;
  onClose: () => void;
  onSaved: (rule: AutomationRule) => void;
  onDuplicate: (copy: AutomationRule) => void;
}

export function RuleEditor({ initial, isNew, accounts, platformAccounts, otherChannelsOn, onClose, onSaved, onDuplicate }: RuleEditorProps) {
  const [stored, setDraft] = useState<AutomationRule>(initial);
  const kickAccount = platformAccounts.find((a) => a.provider === 'kick');
  // A Kick rule that posts sends as the connected Kick account, so its sender follows a connect or a
  // switch; derived here and sent with the save, never set in an effect.
  const draft =
    kickAccount && rulePlatform(stored) === 'kick' && stored.actions.some(isOutward) && stored.send_as.id !== kickAccount.id
      ? { ...stored, send_as: platformAccountRef(kickAccount) }
      : stored;
  const [errors, setErrors] = useState<AutomationFieldError[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [stale, setStale] = useState(false);
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const variables = useVariables();
  const preview = usePreview(draft);

  const dirty = isNew || !sameRuleBody(draft, initial);
  const when = draft.when;
  const badge = when.kind === 'badge_drop';
  const chatTrigger = when.kind === 'message' || when.kind === 'mention';
  const ownEvent = isOwnEventKind(when.kind);
  const edge = isEdgeKind(when.kind);
  const scope = draft.only_if.channels;
  const hours = draft.only_if.hours;
  const hasOutward = draft.actions.some(isOutward);
  const platforms = draft.only_if.platforms;
  const platform = rulePlatform(draft);
  const twitchOnly = platforms.every((p) => p === 'twitch');
  const eventItems = EVENT_ITEMS.filter((item) => eventsOn(platforms).includes(item.id));
  const platformHint = platforms.some((p) => p === 'youtube' || p === 'tiktok')
    ? 'YouTube and TikTok rules can notify you, log or speak.'
    : hasOutward
      ? 'A rule that posts runs on one platform.'
      : 'Choose every platform it should watch.';

  const update = (fn: (r: AutomationRule) => AutomationRule) => {
    setDraft(fn);
    setMessage(null);
  };
  const setIf = (patch: Partial<AutomationConditions>) => update((r) => ({ ...r, only_if: { ...r.only_if, ...patch } }));
  const setLimits = (patch: Partial<AutomationLimits>) => update((r) => ({ ...r, limits: { ...r.limits, ...patch } }));
  const setAction = (index: number, next: AutomationAction) =>
    update((r) => ({ ...r, actions: r.actions.map((a, i) => (i === index ? next : a)) }));
  const moveAction = (index: number, dir: -1 | 1) =>
    update((r) => {
      const target = index + dir;
      if (target < 0 || target >= r.actions.length) return r;
      const actions = r.actions.slice();
      [actions[index], actions[target]] = [actions[target], actions[index]];
      return { ...r, actions };
    });
  const toggleRole = (role: AutomationSenderRole) =>
    setIf({
      roles: draft.only_if.roles.includes(role)
        ? draft.only_if.roles.filter((r) => r !== role)
        : [...draft.only_if.roles, role],
    });
  const addChannel = (login: string, roomId: string, provider: AutomationPlatform) => {
    // Twitch's key is built in Rust; another platform's names it here.
    const key = provider === 'twitch' ? '' : makeKey(provider, login);
    if (scope.kind !== 'these' || scope.list.some((c) => providerOfKey(c.key) === provider && c.name === login)) return;
    setIf({ channels: { kind: 'these', list: [...scope.list, { id: newElementId('c'), key, room_id: roomId, name: login }] } });
  };
  const togglePlatform = (id: AutomationPlatform) => {
    const has = platforms.includes(id);
    // A rule that posts runs on one platform, so a choice replaces; one that only notifies may watch several.
    const next = hasOutward ? [id] : has ? platforms.filter((p) => p !== id) : [...platforms, id];
    update((r) => withPlatforms(r, next, kickAccount));
  };
  const addOutward = (kind: 'reply' | 'send') =>
    update((r) => withPlatforms({ ...r, actions: [...r.actions, actionOf(kind)] }, [rulePlatform(r)], kickAccount));

  const save = () => {
    setSaving(true);
    setMessage(null);
    saveRuleChecked(draft, isNew ? 0 : draft.updated_ms)
      .then(({ rule: sent, answer: res, problems }) => {
        if (!res) {
          setErrors(problems);
          setMessage('Fix the marked fields to save.');
          return;
        }
        if (sent !== draft) setDraft(sent);
        setErrors(res.errors);
        setStale(false);
        if (res.saved) {
          onSaved(res.rule);
          return;
        }
        setDraft(res.rule);
        // Rust writes nothing when there are errors, so a save with errors is never "Saved".
        setMessage('Fix the marked fields to save.');
      })
      .catch((e: unknown) => {
        setMessage(errorText(e));
        setStale(!isNew);
      })
      .finally(() => setSaving(false));
  };

  const remove = () => {
    automationsApi
      .deleteRule(draft.id)
      .then(() => {
        void refreshAutomationStatus();
        onClose();
      })
      .catch((e: unknown) => setMessage(errorText(e)));
  };

  const setLive = (live: boolean) => {
    automationsApi
      .setRuleFlags(initial.id, { live })
      .then(() => void refreshAutomationStatus())
      .catch((e: unknown) => setMessage(errorText(e)));
  };

  const scopeHint =
    scope.kind !== 'own' && !otherChannelsOn
      ? "Rules post in chats that aren't yours only while Other channels is on."
      : scope.kind === 'any_open'
        ? 'Keeps answering in chats you had open when a window closes to the tray.'
        : undefined;

  const signedIn = accounts.some((a) => a.id === draft.send_as.id);
  const ruleError = errorAt(errors, 'rules') ?? errorAt(errors, 'owner');

  return (
    <div className="space-y-3">
      {(message || ruleError) && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-error/30 bg-error/5 px-3 py-2">
          <p className="text-[12px] leading-relaxed text-textSecondary">{message ?? ruleError}</p>
          {stale && (
            <button
              type="button"
              onClick={() => {
                setDraft(initial);
                setStale(false);
                setMessage(null);
                setErrors([]);
              }}
              className={QUIET_BUTTON}
            >
              Use the saved version
            </button>
          )}
        </div>
      )}

      <WhatChanges ruleId={draft.id} />

      <Field label="Name" error={errorAt(errors, 'name')}>
        <input
          value={draft.name}
          maxLength={60}
          aria-label="Name"
          onChange={(e) => update((r) => ({ ...r, name: e.target.value }))}
          placeholder="Away reply"
          className={INPUT}
        />
      </Field>

      <Block title="When">
        <select
          value={when.kind}
          aria-label="When"
          onChange={(e) => update((r) => withTrigger(r, e.target.value as TriggerKind))}
          className={SELECT}
        >
          {TRIGGERS.filter((t) => twitchOnly || t.value !== 'badge_drop').map((t) => (
            <option key={t.value} value={t.value}>
              {t.label}
            </option>
          ))}
        </select>

        {when.kind === 'message' && (
          <>
            <Field label="Match">
              <select
                value={when.text.mode}
                aria-label="Match"
                onChange={(e) =>
                  update((r) => ({ ...r, when: { ...when, text: { ...when.text, mode: e.target.value as AutomationMatchMode } } }))
                }
                className={SELECT}
              >
                {MODES.map((m) => (
                  <option key={m.value} value={m.value}>
                    {m.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Words" error={errorAt(errors, 'when.text.pattern')}>
              <input
                value={when.text.pattern}
                maxLength={200}
                aria-label="Words"
                spellCheck={false}
                onChange={(e) => update((r) => ({ ...r, when: { ...when, text: { ...when.text, pattern: e.target.value } } }))}
                placeholder={when.text.mode === 'regex' ? '^!discord\\b' : 'discord'}
                className={INPUT}
              />
            </Field>
            <SwitchLine
              label="Match case"
              description="Capital letters must match too."
              on={when.text.case_sensitive}
              onChange={() =>
                update((r) => ({ ...r, when: { ...when, text: { ...when.text, case_sensitive: !when.text.case_sensitive } } }))
              }
            />
          </>
        )}

        {when.kind === 'mention' && (
          <SwitchLine
            label="Replies count"
            description="A reply to your message counts as a mention."
            on={when.replies_too}
            onChange={() => update((r) => ({ ...r, when: { ...when, replies_too: !when.replies_too } }))}
          />
        )}

        {when.kind === 'first_message' && (
          <p className={HINT}>Acts on each person's first message in a chat.</p>
        )}

        {when.kind === 'channel_event' && (
          <>
            <Field label="Events" error={errorAt(errors, 'when.events')}>
              <SwitchList
                items={eventItems}
                isOn={(id) => when.events.some((e) => e === id)}
                onToggle={(id) =>
                  update((r) => ({
                    ...r,
                    when: {
                      ...when,
                      events: when.events.some((e) => e === id)
                        ? when.events.filter((e) => e !== id)
                        : [...when.events, ...EVENT_ITEMS.filter((item) => item.id === id).map((item) => item.id)],
                    },
                  }))
                }
              />
            </Field>
            {MINIMUMS.filter((m) => when.events.includes(m.event)).map((m) => (
              <Field key={m.field} label={m.label}>
                <NumberField
                  value={when[m.field]}
                  min={0}
                  max={1_000_000}
                  label={m.label}
                  suffix={m.suffix}
                  onChange={(v) => update((r) => ({ ...r, when: withMinimum(when, m.field, v) }))}
                />
              </Field>
            ))}
          </>
        )}

        {isP2Trigger(when) && <P2TriggerFields when={when} errors={errors} onChange={(next) => update((r) => ({ ...r, when: next }))} />}

        {when.kind === 'badge_drop' && (
          <BadgeDropFields
            trigger={when}
            errors={errors}
            previewLink={preview?.link ?? null}
            onChange={(next) => update((r) => ({ ...r, when: next }))}
          />
        )}

        {ownEvent && (
          <OwnEventFields
            trigger={when}
            ownerId={draft.owner.id}
            errors={errors}
            onChange={(next) => update((r) => ({ ...r, when: next }))}
          />
        )}

        {when.kind === 'live_change' && (
          <LiveChangeFields trigger={when} onChange={(next) => update((r) => ({ ...r, when: next }))} />
        )}
      </Block>

      <Block title="If">
        {badge ? (
          <p className={HINT}>In your channel, only while you're live. Badge rules always work this way.</p>
        ) : edge ? (
          <EdgeIf ownEvent={ownEvent} draft={draft} errors={errors} addChannel={(login, roomId) => addChannel(login, roomId, 'twitch')} setIf={setIf} />
        ) : (
          <>
            <Field label="Platforms" hint={platformHint} error={errorAt(errors, 'only_if.platforms')}>
              <div role="group" aria-label="Platforms">
                {PLATFORM_ITEMS.map((item) => (
                  <div key={item.id} className="flex items-center gap-2.5">
                    <ProviderMark provider={item.id} size={16} />
                    <div className="min-w-0 flex-1">
                      <SwitchLine label={item.label} on={platforms.includes(item.id)} onChange={() => togglePlatform(item.id)} />
                    </div>
                  </div>
                ))}
              </div>
            </Field>
            <Field label="Where" hint={scopeHint} error={errorAt(errors, 'only_if.channels')}>
              <select
                value={scope.kind}
                aria-label="Where"
                onChange={(e) => setIf({ channels: scopeOf(e.target.value as AutomationChannelScope['kind'], scope) })}
                className={SELECT}
              >
                {SCOPES.filter((s) => s.value !== 'watching' || platforms.includes('twitch')).map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
            </Field>
            {scope.kind === 'these' && (
              <div className="space-y-1.5">
                {scope.list.map((c, i) => {
                  const chatError = errorAt(errors, `only_if.channels.list[${i}]`);
                  return (
                    <div key={c.id} className="space-y-1">
                      <div className="flex items-center justify-between gap-2 rounded-md bg-background/40 px-2.5 py-1.5">
                        <span className="flex min-w-0 items-center gap-1.5">
                          {!twitchOnly && <ProviderMark provider={providerOfKey(c.key)} size={13} />}
                          <span className="truncate text-[13px] text-textPrimary">#{c.name || c.key}</span>
                        </span>
                        <button
                          type="button"
                          onClick={() => setIf({ channels: { kind: 'these', list: scope.list.filter((x) => x.id !== c.id) } })}
                          className="rounded p-0.5 text-textMuted transition-colors hover:text-textPrimary"
                          aria-label={`Remove ${c.name || c.key}`}
                        >
                          <X size={13} />
                        </button>
                      </div>
                      {chatError && <p className="text-[12px] text-error">{chatError}</p>}
                    </div>
                  );
                })}
                {scope.list.length < MAX_CHATS && (
                  <ChannelPicker
                    label="Add a channel"
                    providers={platforms}
                    // The picker searches only the platforms named here, so the pick is one of them.
                    onPick={(login, id, provider) => addChannel(login, id, provider as AutomationPlatform)}
                  />
                )}
              </div>
            )}
            <Field label="From" hint="Leave them all off to answer anyone.">
              <SwitchList
                items={ROLE_ITEMS}
                isOn={(id) => draft.only_if.roles.some((r) => r === id)}
                onToggle={(id) => {
                  const role = ROLE_ITEMS.find((item) => item.id === id)?.id;
                  if (role) toggleRole(role);
                }}
              />
            </Field>
            {twitchOnly && (
              <SwitchLine
                label="Only while live"
                description="Only while the stream in that chat is live."
                on={draft.only_if.only_live}
                onChange={() => setIf({ only_live: !draft.only_if.only_live })}
              />
            )}
            <SwitchLine
              label="Only while you're away"
              description={IS_MOBILE ? 'Set away on this page or with /afk.' : 'Set away on this page, with /afk, or from the tray.'}
              on={draft.only_if.only_away}
              onChange={() => setIf({ only_away: !draft.only_if.only_away })}
            />
            <SwitchLine
              label="Only in chats you have open"
              description="Skips chats that only StreamNook is keeping open for rules. Reminders work this way too."
              on={draft.only_if.only_while_open ?? false}
              onChange={() => setIf({ only_while_open: !(draft.only_if.only_while_open ?? false) })}
            />
            <Disclosure label="More conditions">
              <Field label="Skip these people" hint="Names separated by commas.">
                <NamesInput
                  values={draft.only_if.skip_users}
                  label="Skip these people"
                  placeholder="nightbot, streamelements"
                  onCommit={(skip_users) => setIf({ skip_users })}
                />
              </Field>
              <Field label="Only these people" hint="Leave it empty to answer anyone.">
                <NamesInput
                  values={draft.only_if.only_users}
                  label="Only these people"
                  onCommit={(only_users) => setIf({ only_users })}
                />
              </Field>
              {chatTrigger && (
                <SwitchLine
                  label="Shared Chat"
                  description="Also answer messages from the other channels in a Shared Chat."
                  on={draft.only_if.shared_chat}
                  onChange={() => setIf({ shared_chat: !draft.only_if.shared_chat })}
                />
              )}
              <SwitchLine
                label="Only at certain hours"
                description="In your local time."
                on={hours !== null}
                onChange={() => setIf({ hours: hours ? null : { from: '09:00', to: '17:00', days: [] } })}
              />
              {hours && (
                <div className="space-y-3">
                  <div className="flex flex-wrap items-start gap-3">
                    <Field label="From" error={errorAt(errors, 'only_if.hours.from')}>
                      <input
                        type="time"
                        value={hours.from}
                        aria-label="From"
                        onChange={(e) => setIf({ hours: { ...hours, from: e.target.value } })}
                        className={SELECT}
                      />
                    </Field>
                    <Field label="To" error={errorAt(errors, 'only_if.hours.to')}>
                      <input
                        type="time"
                        value={hours.to}
                        aria-label="To"
                        onChange={(e) => setIf({ hours: { ...hours, to: e.target.value } })}
                        className={SELECT}
                      />
                    </Field>
                  </div>
                  <Field label="Days" hint="Leave them all off for every day. A To before From runs past midnight.">
                    <SwitchList
                      items={DAYS}
                      isOn={(id) => hours.days.some((d) => d === id)}
                      onToggle={(id) => {
                        const day = DAYS.find((d) => d.id === id)?.id;
                        if (!day) return;
                        setIf({
                          hours: {
                            ...hours,
                            days: hours.days.includes(day) ? hours.days.filter((d) => d !== day) : [...hours.days, day],
                          },
                        });
                      }}
                    />
                  </Field>
                </div>
              )}
            </Disclosure>
          </>
        )}
        <Disclosure label="Pace and cooldowns">
          {!badge && (
            <Field
              label="Chat cooldown"
              hint="Time between two of its answers in one chat."
              error={errorAt(errors, 'limits.rule_cooldown_seconds')}
            >
              <NumberField
                value={draft.limits.rule_cooldown_seconds}
                min={0}
                max={2_592_000}
                label="Chat cooldown"
                suffix="seconds"
                onChange={(v) => setLimits({ rule_cooldown_seconds: v })}
              />
            </Field>
          )}
          {!badge && (
            <Field
              label="Person cooldown"
              hint="Time before it answers the same person again."
              error={errorAt(errors, 'limits.user_cooldown_seconds')}
            >
              <NumberField
                value={draft.limits.user_cooldown_seconds}
                min={0}
                max={2_592_000}
                label="Person cooldown"
                suffix="seconds"
                onChange={(v) => setLimits({ user_cooldown_seconds: v })}
              />
            </Field>
          )}
          <Field label="Most an hour" error={errorAt(errors, 'limits.max_per_hour')}>
            <NumberField
              value={draft.limits.max_per_hour}
              min={1}
              max={120}
              label="Most an hour"
              suffix="times"
              onChange={(v) => setLimits({ max_per_hour: v })}
            />
          </Field>
          <Field
            label="Longest wait"
            hint="How long it may wait for a free moment to send before it skips."
            error={errorAt(errors, 'limits.max_wait_seconds')}
          >
            <NumberField
              value={draft.limits.max_wait_seconds}
              min={0}
              max={130}
              label="Longest wait"
              suffix="seconds"
              onChange={(v) => setLimits({ max_wait_seconds: v })}
            />
          </Field>
          {hasOutward && (
            <SwitchLine
              label="Vary repeats"
              description="Adds an invisible mark so Twitch takes the same line twice in a row."
              on={draft.vary}
              onChange={() => update((r) => ({ ...r, vary: !r.vary }))}
            />
          )}
        </Disclosure>
      </Block>

      <Block
        title="Do"
        aside={
          <span className="text-[11.5px] text-textMuted">
            {draft.actions.length} of {MAX_ACTIONS}
          </span>
        }
      >
        {errorAt(errors, 'actions') && <p className="text-[12px] text-error">{errorAt(errors, 'actions')}</p>}
        {draft.actions.map((a, i) => (
          <ActionEditor
            key={a.id}
            action={a}
            index={i}
            count={draft.actions.length}
            badgeRule={badge}
            timedOwn={(draft.when.kind === 'timer' || draft.when.kind === 'clock') && draft.only_if.channels.kind === 'own'}
            platform={platform}
            errors={errors}
            variables={variables}
            preview={preview?.texts.find((t) => t.action_id === a.id)?.text ?? null}
            onChange={(next) => setAction(i, next)}
            onRemove={() => update((r) => ({ ...r, actions: r.actions.filter((_, j) => j !== i) }))}
            onMove={(dir) => moveAction(i, dir)}
          />
        ))}
        {draft.actions.length < MAX_ACTIONS && (
          <div className="flex flex-wrap items-center gap-1.5">
            {!badge && !edge && platforms.every(canPost) && (
              <button type="button" onClick={() => addOutward('reply')} className={QUIET_BUTTON}>
                <Plus size={13} />
                Reply
              </button>
            )}
            {platforms.every(canPost) && (
              <button type="button" onClick={() => addOutward('send')} className={QUIET_BUTTON}>
                <Plus size={13} />
                Post a message
              </button>
            )}
            <button type="button" onClick={() => update((r) => ({ ...r, actions: [...r.actions, actionOf('notify')] }))} className={QUIET_BUTTON}>
              <Plus size={13} />
              Notify me
            </button>
            <button type="button" onClick={() => update((r) => ({ ...r, actions: [...r.actions, actionOf('speak')] }))} className={QUIET_BUTTON}>
              <Plus size={13} />
              Say it out loud
            </button>
            {when.kind === 'message' && (
              <button type="button" onClick={() => update((r) => ({ ...r, actions: [...r.actions, actionOf('highlight')] }))} className={QUIET_BUTTON}>
                <Plus size={13} />
                Highlight
              </button>
            )}
            {!badge && (
              <P2AddActions
                rule={draft}
                sendAsIsMain={accounts.some((a) => a.id === draft.send_as.id && a.main)}
                onAdd={(next) => update((r) => ({ ...r, actions: [...r.actions, next] }))}
              />
            )}
          </div>
        )}
      </Block>

      {(draft.when.kind === 'message' || draft.when.kind === 'mention' || draft.when.kind === 'first_message' || draft.when.kind === 'channel_event') && (
        <Block title="Advanced">
          <ExpressionField
            value={draft.only_if.expression}
            errors={errors}
            onChange={(expression) => setIf({ expression })}
          />
        </Block>
      )}

      {hasOutward && platform === 'kick' && (
        <Block title="Send as">
          <div className="flex items-center gap-2 rounded-md bg-background/40 px-2.5 py-2">
            <ProviderMark provider="kick" size={14} />
            <span className="text-[13px] text-textPrimary">{kickAccount?.login || draft.send_as.login || 'Kick'}</span>
          </div>
          <p className={HINT}>
            {kickAccount
              ? 'Your connected Kick account. Its pace and cooldowns count on it.'
              : 'Connect your Kick account in Settings to run this rule.'}
          </p>
        </Block>
      )}

      {hasOutward && platform !== 'kick' && (
        <Block title="Send as">
          <select
            value={draft.send_as.id}
            aria-label="Send as"
            onChange={(e) => {
              const account = accounts.find((a) => a.id === e.target.value);
              if (account) update((r) => ({ ...r, send_as: accountRef(account) }));
            }}
            className={SELECT}
          >
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.login}
                {a.main ? ' (main account)' : ''}
              </option>
            ))}
            {!signedIn && (
              <option value={draft.send_as.id}>
                {draft.send_as.name || draft.send_as.login} (signed out)
              </option>
            )}
          </select>
          <p className={HINT}>Its pace and cooldowns count on the account that speaks.</p>
        </Block>
      )}

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-borderSubtle/60 pt-3">
        <div className="flex items-center gap-2">
          {isNew ? (
            <span className={HINT}>New rules start in Test.</span>
          ) : (
            <SegmentedSelect
              value={initial.live ? 'live' : 'test'}
              options={[
                { value: 'test', label: 'Test' },
                { value: 'live', label: 'Live' },
              ]}
              onChange={(v) => setLive(v === 'live')}
            />
          )}
        </div>
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          {!isNew && (
            <button type="button" onClick={() => onDuplicate(draft)} className={QUIET_BUTTON}>
              <Copy size={13} />
              Duplicate
            </button>
          )}
          {!isNew &&
            (confirmDelete ? (
              <>
                <button type="button" onClick={remove} className={DANGER_BUTTON}>
                  Delete rule
                </button>
                <button type="button" onClick={() => setConfirmDelete(false)} className={QUIET_BUTTON}>
                  Keep it
                </button>
              </>
            ) : (
              <button type="button" onClick={() => setConfirmDelete(true)} className={QUIET_BUTTON}>
                <Trash2 size={13} />
                Delete
              </button>
            ))}
          <button type="button" onClick={onClose} className={QUIET_BUTTON}>
            {dirty ? 'Discard changes' : 'Close'}
          </button>
          <button type="button" onClick={save} disabled={saving || !dirty} className={PRIMARY_BUTTON}>
            {saving ? 'Saving' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  );
}
