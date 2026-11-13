// A badge rule's When: which moment posts, the link and its check, what to do
// when the link is not ready, and the one line for a batch.
import { useState } from 'react';
import type { AutomationBadgeLeg, AutomationBadgeLink, AutomationFieldError, AutomationTrigger } from '../../../types';
import { automationsApi, errorAt, errorText, LINK_NOT_READY, LINK_READY } from '../../../utils/automations';
import { IS_MOBILE } from '../../../utils/platform';
import { SegmentedSelect } from '../_primitives';
import { Field, HINT, INPUT, QUIET_BUTTON, SELECT } from './parts';

type BadgeTrigger = Extract<AutomationTrigger, { kind: 'badge_drop' }>;

const LEGS: { value: AutomationBadgeLeg; label: string }[] = [
  { value: 'announced', label: 'Announced' },
  { value: 'earnable', label: 'Earnable' },
  { value: 'both', label: 'Both' },
];

const LEG_WORDS: Record<AutomationBadgeLeg, string> = {
  announced: 'Posts when Twitch first shows a new badge.',
  earnable: 'Posts when a badge can be earned.',
  both: "Posts when it's announced, and again when it can be earned.",
};

const PRESETS: { value: AutomationBadgeLink['preset']; label: string }[] = [
  { value: 'streamnook', label: 'StreamNook badge page' },
  { value: 'streamdatabase', label: 'Stream Database' },
  { value: 'custom', label: 'My own link' },
  { value: 'none', label: 'No link' },
];

export function BadgeDropFields({
  trigger,
  errors,
  previewLink,
  onChange,
}: {
  trigger: BadgeTrigger;
  errors: readonly AutomationFieldError[];
  /** The filled link for the newest drop, from the live preview. */
  previewLink: string | null;
  onChange: (next: BadgeTrigger) => void;
}) {
  const [check, setCheck] = useState<{ link: string; ready: boolean; message: string } | null>(null);
  const [checking, setChecking] = useState(false);
  const setLink = (patch: Partial<AutomationBadgeLink>) => onChange({ ...trigger, link: { ...trigger.link, ...patch } });

  const runCheck = () => {
    if (!previewLink) return;
    const link = previewLink;
    setChecking(true);
    automationsApi
      .checkLink(link)
      .then((ready) => setCheck({ link, ready, message: ready ? LINK_READY : LINK_NOT_READY }))
      .catch((e: unknown) => setCheck({ link, ready: false, message: errorText(e) }))
      .finally(() => setChecking(false));
  };
  // An answer about a link the preview no longer shows says nothing.
  const answer = check && check.link === previewLink ? check : null;

  return (
    <div className="space-y-3">
      <Field label="Posts when" hint={LEG_WORDS[trigger.on]}>
        <SegmentedSelect
          value={trigger.on}
          options={LEGS}
          onChange={(on) => onChange({ ...trigger, on })}
          fullWidth={IS_MOBILE}
        />
      </Field>
      <Field label="Link" error={errorAt(errors, 'when.link.custom')}>
        <select
          value={trigger.link.preset}
          aria-label="Link"
          onChange={(e) => setLink({ preset: e.target.value as AutomationBadgeLink['preset'] })}
          className={SELECT}
        >
          {PRESETS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label}
            </option>
          ))}
        </select>
        {trigger.link.preset === 'custom' && (
          <div className="space-y-1 pt-1">
            <input
              value={trigger.link.custom}
              aria-label="Your link"
              onChange={(e) => setLink({ custom: e.target.value })}
              placeholder="https://example.com/badges/{badge.id}"
              spellCheck={false}
              className={INPUT}
            />
            <p className={HINT}>Fill in the badge with {'{badge.id}'} and {'{badge.version}'}.</p>
          </div>
        )}
      </Field>
      {trigger.link.preset !== 'none' && previewLink && (
        <div className="space-y-1.5 rounded-md bg-background/40 px-2.5 py-2">
          <span className="block text-[11px] text-textMuted">For the newest badge</span>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="min-w-0 break-all font-mono text-[11.5px] text-textSecondary">{previewLink}</span>
            <button type="button" onClick={runCheck} disabled={checking} className={QUIET_BUTTON}>
              {checking ? 'Checking' : 'Check it'}
            </button>
          </div>
          {answer && (
            <p className={`text-[12px] leading-relaxed ${answer.ready ? 'text-textSecondary' : 'text-warning'}`}>
              {answer.message}
            </p>
          )}
        </div>
      )}
      {trigger.link.preset !== 'none' && (
        <Field label="If the link isn't ready after 10 minutes">
          <select
            value={trigger.link.if_not_ready}
            aria-label="If the link isn't ready after 10 minutes"
            onChange={(e) => setLink({ if_not_ready: e.target.value as AutomationBadgeLink['if_not_ready'] })}
            className={SELECT}
          >
            <option value="streamnook">Post the StreamNook link</option>
            <option value="no_link">Post without a link</option>
          </select>
        </Field>
      )}
      <Field
        label="When several drop together"
        hint="One message for the whole batch. When one drops alone, your Post a message lines are used."
        error={errorAt(errors, 'when.batch_text')}
      >
        <textarea
          value={trigger.batch_text}
          rows={2}
          maxLength={500}
          aria-label="When several drop together"
          onChange={(e) => onChange({ ...trigger, batch_text: e.target.value })}
          placeholder="{badge.count} new Twitch badges just dropped: {badges}!"
          className={`${INPUT} resize-y`}
        />
      </Field>
    </div>
  );
}
