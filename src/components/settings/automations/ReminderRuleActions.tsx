// "Make it a rule" and "Turn back on": the two buttons a reminder gets, on the
// Reminders section and in the Automations group. Rust does the work; these call
// it and say what came back. A refusal ("Already a rule") shows under the button.
import { useState } from 'react';
import { useAppStore } from '../../../stores/AppStore';
import { useAutomationStatusStore } from '../../../hooks/useAutomationStatus';
import { errorText } from '../../../utils/automations';
import { convertReminder, restoreReminder, ruleMadeFrom } from '../../../utils/reminderRules';
import { QUIET_BUTTON } from './parts';

const PROBLEM = 'max-w-xs text-right text-[11.5px] leading-snug text-error';

/** Builds a rule from the reminder, in Test, and opens its editor with "What changes". */
export function MakeRuleButton({ reminderId }: { reminderId: string }) {
  const openSettings = useAppStore((s) => s.openSettings);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const run = () => {
    setBusy(true);
    setProblem(null);
    convertReminder(reminderId)
      .then((answer) => openSettings('Automations', undefined, `rule:${answer.rule_id}`))
      .catch((e: unknown) => setProblem(errorText(e)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="flex flex-col items-end gap-1">
      <button
        type="button"
        onClick={run}
        disabled={busy}
        title="Build a rule from this reminder, in Test. The reminder switches off, and you can turn it back on."
        className={QUIET_BUTTON}
      >
        Make it a rule
      </button>
      {problem && (
        <p role="alert" className={PROBLEM}>
          {problem}
        </p>
      )}
    </div>
  );
}

/** A reminder that is a rule now: where it went, and the way back. `compact` is the bare row for a list. */
export function ConvertedNote({ reminderId, compact = false }: { reminderId: string; compact?: boolean }) {
  const openSettings = useAppStore((s) => s.openSettings);
  const ruleId = useAutomationStatusStore((s) => ruleMadeFrom(s.status, reminderId));
  const [problem, setProblem] = useState<string | null>(null);

  const back = () => {
    setProblem(null);
    restoreReminder(reminderId).catch((e: unknown) => setProblem(errorText(e)));
  };

  return (
    <div className="flex flex-col items-end gap-1">
      <div
        className={
          compact
            ? 'flex items-center gap-2'
            : 'flex w-full flex-wrap items-center justify-between gap-2 rounded-md bg-background/40 px-2.5 py-1.5'
        }
      >
        <span className="text-[12px] text-textSecondary">Now a rule</span>
        <span className="flex items-center gap-2">
          {ruleId && (
            <button
              type="button"
              onClick={() => openSettings('Automations', undefined, `rule:${ruleId}`)}
              className={QUIET_BUTTON}
            >
              Open rule
            </button>
          )}
          <button type="button" onClick={back} className={QUIET_BUTTON}>
            Turn back on
          </button>
        </span>
      </div>
      {problem && (
        <p role="alert" className={PROBLEM}>
          {problem}
        </p>
      )}
    </div>
  );
}
