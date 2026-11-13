// The rules: one row each (name, what it does, its switch, Test or Live, who
// it sends as, its record), one editor open at a time, reorder, Add rule and
// the starter templates.
import { useState } from 'react';
import { ArrowDown, ArrowUp, ChevronDown, LayoutTemplate, Plus, Trash2 } from 'lucide-react';
import type { AutomationRule } from '../../../types';
import { Toggle } from '../../ui/Toggle';
import { IS_MOBILE } from '../../../utils/platform';
import { refreshAutomationStatus } from '../../../hooks/useAutomationStatus';
import {
  accountRef,
  actedText,
  automationsApi,
  blankRule,
  duplicateRule,
  errorText,
  moveBeforeId,
  type AutomationAccount,
  type AutomationRuleStatus,
  type AutomationStatus,
  type AutomationTemplate,
} from '../../../utils/automations';
import { RuleEditor } from './RuleEditor';
import { AccountFace, DANGER_BUTTON, HINT, LiveWord, PRIMARY_BUTTON, QUIET_BUTTON } from './parts';

/** The open editor of a rule not saved yet. */
const NEW = 'new';

function RuleRow({
  r,
  index,
  total,
  open,
  now,
  accounts,
  confirming,
  onOpen,
  onFlip,
  onMove,
  onAskDelete,
  onDelete,
  onKeep,
}: {
  r: AutomationRuleStatus;
  index: number;
  total: number;
  open: boolean;
  now: number;
  accounts: readonly AutomationAccount[];
  confirming: boolean;
  onOpen: () => void;
  onFlip: () => void;
  onMove: (dir: -1 | 1) => void;
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
  return (
    <div className={`flex items-start gap-3 px-3.5 py-3 ${r.enabled ? '' : 'opacity-70'}`}>
      <div className="pt-0.5">
        <Toggle
          enabled={r.enabled}
          onChange={onFlip}
          disabled={!rule}
          ariaLabel={r.enabled ? `Turn off ${r.name}` : `Turn on ${r.name}`}
        />
      </div>
      <button type="button" onClick={onOpen} disabled={!rule} aria-expanded={open} className="min-w-0 flex-1 text-left">
        <span className="flex items-center gap-2">
          <span className="truncate text-[13.5px] font-medium text-textPrimary">{r.name || 'Untitled rule'}</span>
          {speaker && <AccountFace account={speaker} />}
        </span>
        {r.summary && <span className="mt-0.5 block text-[12px] leading-relaxed text-textSecondary">{r.summary}</span>}
        <span className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px] text-textMuted">
          <LiveWord live={r.live} />
          <span>{actedText(r.count, r.last_fire_ms, now)}</span>
          {r.last_outcome && <span>{r.last_outcome}</span>}
        </span>
        {r.held && <span className="mt-1.5 block text-[12px] leading-relaxed text-warning">{r.held}</span>}
      </button>
      <div className="flex shrink-0 items-center gap-0.5">
        <button
          type="button"
          onClick={() => onMove(-1)}
          disabled={index === 0}
          className="rounded p-1 text-textSecondary transition-colors hover:text-textPrimary disabled:cursor-not-allowed disabled:opacity-30"
          aria-label="Move up"
        >
          <ArrowUp size={14} />
        </button>
        <button
          type="button"
          onClick={() => onMove(1)}
          disabled={index === total - 1}
          className="rounded p-1 text-textSecondary transition-colors hover:text-textPrimary disabled:cursor-not-allowed disabled:opacity-30"
          aria-label="Move down"
        >
          <ArrowDown size={14} />
        </button>
        {rule ? (
          <button
            type="button"
            onClick={onOpen}
            className="rounded p-1 text-textSecondary transition-colors hover:text-textPrimary"
            aria-label={open ? 'Close the editor' : 'Edit this rule'}
          >
            <ChevronDown size={14} className={`transition-transform ${open ? 'rotate-180' : ''}`} />
          </button>
        ) : confirming ? (
          <span className="flex items-center gap-1 pl-1">
            <button type="button" onClick={onDelete} className={DANGER_BUTTON}>
              Delete
            </button>
            <button type="button" onClick={onKeep} className={QUIET_BUTTON}>
              Keep
            </button>
          </span>
        ) : (
          <button
            type="button"
            onClick={onAskDelete}
            className="rounded p-1 text-textSecondary transition-colors hover:text-error"
            aria-label="Delete this rule"
          >
            <Trash2 size={14} />
          </button>
        )}
      </div>
    </div>
  );
}

interface RuleListProps {
  status: AutomationStatus | null;
  now: number;
  /** A rule to open first (a settings link named it). */
  initialOpen: string | null;
  otherChannelsOn: boolean;
}

export default function RuleList({ status, now, initialOpen, otherChannelsOn }: RuleListProps) {
  const [openId, setOpenId] = useState<string | null>(initialOpen);
  const [fresh, setFresh] = useState<{ rule: AutomationRule; n: number } | null>(null);
  const [showTemplates, setShowTemplates] = useState(false);
  const [templates, setTemplates] = useState<AutomationTemplate[] | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  if (!status) return <p className={`${HINT} ${IS_MOBILE ? 'px-4' : 'px-1'}`}>Loading your rules</p>;

  const rules = status.rules;
  const ids = rules.map((r) => r.id);
  const main = status.accounts.find((a) => a.main) ?? null;
  const fail = (e: unknown) => setProblem(errorText(e));
  const done = () => {
    setProblem(null);
    void refreshAutomationStatus();
  };

  const openNew = (rule: AutomationRule) => {
    setFresh((f) => ({ rule, n: (f?.n ?? 0) + 1 }));
    setOpenId(NEW);
    setShowTemplates(false);
  };
  const startBlank = () => {
    if (main) openNew(blankRule(accountRef(main)));
  };
  const toggleTemplates = () => {
    setShowTemplates((v) => !v);
    if (templates === null) automationsApi.templates().then(setTemplates).catch(fail);
  };
  const addTemplate = (id: string) => {
    setAdding(id);
    automationsApi
      .addTemplate(id)
      .then((answer) => {
        setShowTemplates(false);
        setOpenId(answer.rule.id);
        done();
      })
      .catch(fail)
      .finally(() => setAdding(null));
  };
  const move = (index: number, dir: -1 | 1) => {
    const to = moveBeforeId(ids, index, dir);
    if (to) automationsApi.moveRule(ids[index], to.beforeId).then(done).catch(fail);
  };
  const flip = (r: AutomationRuleStatus) => {
    automationsApi.setRuleFlags(r.id, { enabled: !r.enabled }).then(done).catch(fail);
  };
  const remove = (id: string) => {
    setConfirmId(null);
    automationsApi.deleteRule(id).then(done).catch(fail);
  };
  const saved = () => {
    setOpenId(null);
    done();
  };

  return (
    <div className="space-y-2.5">
      {problem && (
        <p role="alert" className={`text-[12px] text-error ${IS_MOBILE ? 'px-4' : 'px-1'}`}>
          {problem}
        </p>
      )}

      {rules.length === 0 && openId !== NEW && (
        <div className="rounded-lg bg-glass/30 px-4 py-6 text-center">
          <p className="text-sm text-textSecondary">
            {main ? 'No rules yet. Start from a template, or make your own.' : 'Sign in with Twitch to make rules.'}
          </p>
        </div>
      )}

      {rules.map((r, index) => (
        <div key={r.id} className={`rounded-lg bg-glass/30 ${openId === r.id ? 'ring-1 ring-borderSubtle' : ''}`}>
          <RuleRow
            r={r}
            index={index}
            total={rules.length}
            open={openId === r.id}
            now={now}
            accounts={status.accounts}
            confirming={confirmId === r.id}
            onOpen={() => setOpenId((id) => (id === r.id ? null : r.id))}
            onFlip={() => flip(r)}
            onMove={(dir) => move(index, dir)}
            onAskDelete={() => setConfirmId(r.id)}
            onDelete={() => remove(r.id)}
            onKeep={() => setConfirmId(null)}
          />
          {openId === r.id && r.rule && (
            <div className="border-t border-borderSubtle/60 px-3.5 pb-3.5 pt-3">
              <RuleEditor
                key={r.id}
                initial={r.rule}
                isNew={false}
                accounts={status.accounts}
                platformAccounts={status.platform_accounts}
                otherChannelsOn={otherChannelsOn}
                onClose={() => setOpenId(null)}
                onSaved={saved}
                onDuplicate={(rule) => openNew(duplicateRule(rule))}
              />
            </div>
          )}
        </div>
      ))}

      {openId === NEW && fresh && (
        <div className="rounded-lg bg-glass/30 ring-1 ring-borderSubtle">
          <div className="px-3.5 pt-3 text-[13.5px] font-medium text-textPrimary">New rule</div>
          <div className="px-3.5 pb-3.5 pt-2.5">
            <RuleEditor
              key={`new-${fresh.n}`}
              initial={fresh.rule}
              isNew
              accounts={status.accounts}
              platformAccounts={status.platform_accounts}
              otherChannelsOn={otherChannelsOn}
              onClose={() => setOpenId(null)}
              onSaved={saved}
              onDuplicate={(rule) => openNew(duplicateRule(rule))}
            />
          </div>
        </div>
      )}

      {main && (
        <div className={`flex flex-wrap items-center gap-2 ${IS_MOBILE ? 'px-1' : ''}`}>
          <button type="button" onClick={startBlank} className={PRIMARY_BUTTON}>
            <Plus size={14} />
            Add rule
          </button>
          <button type="button" onClick={toggleTemplates} aria-expanded={showTemplates} className={QUIET_BUTTON}>
            <LayoutTemplate size={14} />
            From a template
            <ChevronDown size={13} className={`transition-transform ${showTemplates ? 'rotate-180' : ''}`} />
          </button>
        </div>
      )}

      {showTemplates && (
        <div className="space-y-1 rounded-lg border border-borderSubtle/70 bg-white/[0.03] p-3">
          {templates === null ? (
            <p className={HINT}>Loading templates</p>
          ) : (
            templates.map((t) => (
              <div key={t.id} className="flex items-center justify-between gap-3 rounded-md px-1 py-2">
                <div className="min-w-0">
                  <div className="text-[13px] font-medium text-textPrimary">{t.name}</div>
                  <div className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">{t.description}</div>
                </div>
                <button
                  type="button"
                  onClick={() => addTemplate(t.id)}
                  disabled={adding !== null}
                  className={PRIMARY_BUTTON}
                >
                  {adding === t.id ? 'Adding' : 'Add'}
                </button>
              </div>
            ))
          )}
          <p className={`${HINT} px-1 pt-1`}>Templates start in Test, so you can see what they would do first.</p>
        </div>
      )}
    </div>
  );
}
