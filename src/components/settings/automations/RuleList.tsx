// The rules: one card, one row each (what it waits for, its name with Live or
// Test, what it does, its record, anything holding it back), its ⋯ menu and its
// switch at the right edge. A row opens the rule's editor, which the page shows
// in place of the list. The heading's Templates and New rule live here too.
import { useEffect, useState } from 'react';
import { ArrowDown, ArrowUp, CircleHelp, Copy, LayoutTemplate, Plus, Trash2 } from 'lucide-react';
import type { AutomationRule } from '../../../types';
import { Toggle } from '../../ui/Toggle';
import { IS_MOBILE } from '../../../utils/platform';
import { refreshAutomationStatus } from '../../../hooks/useAutomationStatus';
import {
  actedText,
  automationsApi,
  errorText,
  moveBeforeId,
  type AutomationAccount,
  type AutomationRuleStatus,
  type AutomationStatus,
  type AutomationTemplate,
} from '../../../utils/automations';
import { TRIGGER_ICONS } from './labels';
import {
  AccountFace,
  CARD,
  DANGER_BUTTON,
  ErrorLine,
  GHOST_BUTTON,
  HINT,
  META,
  MenuButton,
  Note,
  PRIMARY_BUTTON,
  QUIET_BUTTON,
  RuleTag,
  WarnLine,
  type MenuItem,
} from './parts';

/** The column the row's mark sits in; the hairlines and helper lines keep to the text after it. */
const LEAD = IS_MOBILE ? 'w-6' : 'w-[18px]';

function RuleRow({
  r,
  index,
  total,
  now,
  accounts,
  confirming,
  problem,
  onOpen,
  onFlip,
  onMove,
  onDuplicate,
  onAskDelete,
  onDelete,
  onKeep,
}: {
  r: AutomationRuleStatus;
  index: number;
  total: number;
  now: number;
  accounts: readonly AutomationAccount[];
  confirming: boolean;
  problem: string | null;
  onOpen: () => void;
  onFlip: () => void;
  onMove: (dir: -1 | 1) => void;
  onDuplicate: () => void;
  onAskDelete: () => void;
  onDelete: () => void;
  onKeep: () => void;
}) {
  const rule = r.rule;
  // Its face shows only when another account speaks for the owner.
  const speaker =
    rule && rule.send_as.id !== rule.owner.id
      ? (accounts.find((a) => a.id === rule.send_as.id) ?? rule.send_as)
      : null;
  const Mark = rule ? TRIGGER_ICONS[rule.when.kind] : CircleHelp;
  const menu: MenuItem[] = [
    { id: 'up', label: 'Move up', icon: ArrowUp, disabled: index === 0, onSelect: () => onMove(-1) },
    { id: 'down', label: 'Move down', icon: ArrowDown, disabled: index === total - 1, onSelect: () => onMove(1) },
    ...(rule ? [{ id: 'dup', label: 'Duplicate', icon: Copy, onSelect: onDuplicate }] : []),
    { id: 'delete', label: 'Delete', icon: Trash2, danger: true, onSelect: onAskDelete },
  ];
  return (
    <div className="flex items-start gap-3">
      <span className={`${LEAD} flex shrink-0 justify-center pt-[2px] ${r.enabled ? 'text-accent-on-surface' : 'text-textMuted'}`}>
        <Mark size={IS_MOBILE ? 20 : 17} aria-hidden />
      </span>
      <button
        type="button"
        onClick={onOpen}
        disabled={!rule}
        aria-label={rule ? `Edit ${r.name || 'this rule'}` : undefined}
        className="min-w-0 flex-1 text-left disabled:cursor-default"
      >
        <span className="flex min-w-0 items-center gap-2">
          <span
            className={`truncate ${IS_MOBILE ? 'text-[15px]' : 'text-[14px]'} font-medium ${r.enabled ? 'text-textPrimary' : 'text-textSecondary'}`}
          >
            {r.name || 'Untitled rule'}
          </span>
          <RuleTag live={r.live} />
          {speaker && <AccountFace account={speaker} />}
        </span>
        {r.summary && <span className={`mt-0.5 block ${HINT}`}>{r.summary}</span>}
        <span className={`mt-1 flex flex-wrap gap-x-3 gap-y-0.5 ${META}`}>
          {!r.enabled && <span>Off</span>}
          {!rule && <span>Made by a newer StreamNook. Update to edit it.</span>}
          <span>{actedText(r.count, r.last_fire_ms, now)}</span>
          {r.last_outcome && <span>{r.last_outcome}</span>}
        </span>
        {r.held && <WarnLine className="mt-1.5">{r.held}</WarnLine>}
        {problem && <ErrorLine className="mt-1.5">{problem}</ErrorLine>}
      </button>
      {confirming ? (
        <span className="flex shrink-0 items-center gap-1.5">
          <button type="button" onClick={onDelete} className={DANGER_BUTTON}>
            Delete
          </button>
          <button type="button" onClick={onKeep} className={QUIET_BUTTON}>
            Keep
          </button>
        </span>
      ) : (
        <span className="flex shrink-0 items-center gap-1.5">
          <MenuButton label={`${r.name || 'Rule'} options`} items={menu} />
          <Toggle
            enabled={r.enabled}
            onChange={onFlip}
            disabled={!rule}
            ariaLabel={r.enabled ? `Turn off ${r.name}` : `Turn on ${r.name}`}
          />
        </span>
      )}
    </div>
  );
}

interface RuleListProps {
  status: AutomationStatus | null;
  now: number;
  onOpen: (id: string) => void;
  onDuplicate: (rule: AutomationRule) => void;
}

export default function RuleList({ status, now, onOpen, onDuplicate }: RuleListProps) {
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [problem, setProblem] = useState<{ ruleId: string; text: string } | null>(null);

  if (!status) return <p className={`${HINT} ${IS_MOBILE ? 'px-4' : 'px-1'}`}>Loading your rules</p>;

  const rules = status.rules;
  const ids = rules.map((r) => r.id);
  const main = status.accounts.find((a) => a.main) ?? null;
  const fail = (ruleId: string) => (e: unknown) => setProblem({ ruleId, text: errorText(e) });
  const done = () => {
    setProblem(null);
    void refreshAutomationStatus();
  };

  const move = (index: number, dir: -1 | 1) => {
    const to = moveBeforeId(ids, index, dir);
    if (to) automationsApi.moveRule(ids[index], to.beforeId).then(done).catch(fail(ids[index]));
  };
  const flip = (r: AutomationRuleStatus) => {
    automationsApi.setRuleFlags(r.id, { enabled: !r.enabled }).then(done).catch(fail(r.id));
  };
  const remove = (id: string) => {
    setConfirmId(null);
    automationsApi.deleteRule(id).then(done).catch(fail(id));
  };

  if (rules.length === 0) {
    return (
      <div className={`${CARD} px-4 py-6 text-center`}>
        <p className="text-[13px] text-textSecondary">
          {main ? 'No rules yet. Start from a template, or make your own with New rule.' : 'Sign in with Twitch to make rules.'}
        </p>
      </div>
    );
  }

  return (
    <div className={`${CARD} automation-rows`}>
      {rules.map((r, index) => (
        <RuleRow
          key={r.id}
          r={r}
          index={index}
          total={rules.length}
          now={now}
          accounts={status.accounts}
          confirming={confirmId === r.id}
          problem={problem?.ruleId === r.id ? problem.text : null}
          onOpen={() => onOpen(r.id)}
          onFlip={() => flip(r)}
          onMove={(dir) => move(index, dir)}
          onDuplicate={() => r.rule && onDuplicate(r.rule)}
          onAskDelete={() => setConfirmId(r.id)}
          onDelete={() => remove(r.id)}
          onKeep={() => setConfirmId(null)}
        />
      ))}
    </div>
  );
}

/** Templates and New rule, on the Rules heading's line. */
export function RuleHeaderActions({
  canAdd,
  templatesOpen,
  onTemplates,
  onNew,
}: {
  canAdd: boolean;
  templatesOpen: boolean;
  onTemplates: () => void;
  onNew: () => void;
}) {
  if (!canAdd) return null;
  return (
    <>
      <button type="button" onClick={onTemplates} aria-expanded={templatesOpen} className={GHOST_BUTTON}>
        <LayoutTemplate size={14} />
        Templates
      </button>
      <button type="button" onClick={onNew} className={PRIMARY_BUTTON}>
        <Plus size={14} />
        New rule
      </button>
    </>
  );
}

/** The starter rules, one row each with Add. */
export function TemplatePicker({ onAdded }: { onAdded: (ruleId: string) => void }) {
  const [templates, setTemplates] = useState<AutomationTemplate[] | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    automationsApi
      .templates()
      .then((list) => live && setTemplates(list))
      .catch((e: unknown) => live && setProblem(errorText(e)));
    return () => {
      live = false;
    };
  }, []);

  const add = (id: string) => {
    setAdding(id);
    automationsApi
      .addTemplate(id)
      .then((answer) => {
        void refreshAutomationStatus();
        onAdded(answer.rule.id);
      })
      .catch((e: unknown) => setProblem(errorText(e)))
      .finally(() => setAdding(null));
  };

  return (
    <div className="space-y-2">
      <div className={`${CARD} automation-rows`}>
        {templates === null && !problem && <p className={HINT}>Loading templates</p>}
        {(templates ?? []).map((t) => (
          <div key={t.id} className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="text-[13.5px] font-medium text-textPrimary">{t.name}</div>
              <div className={`mt-0.5 ${HINT}`}>{t.description}</div>
            </div>
            <button type="button" onClick={() => add(t.id)} disabled={adding !== null} className={QUIET_BUTTON}>
              {adding === t.id ? 'Adding' : 'Add'}
            </button>
          </div>
        ))}
        {problem && <ErrorLine>{problem}</ErrorLine>}
      </div>
      <Note className={IS_MOBILE ? 'px-4' : 'px-1'}>Templates start in Test, so you can see what they would do first.</Note>
    </div>
  );
}
