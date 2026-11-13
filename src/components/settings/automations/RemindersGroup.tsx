// The Reminders group on the Automations page: every reminder in one list, with
// its schedule, chat, switch and when it last posted. Reminders stay reminders:
// "Edit in Reminders" opens their own section, and "Make it a rule" is the only
// way one joins the rules. The page draws the section around this (the settings
// search reads sections from AutomationsSettings.tsx only).
import { useState } from 'react';
import { useAppStore } from '../../../stores/AppStore';
import { Toggle } from '../../ui/Toggle';
import { refreshAutomationStatus } from '../../../hooks/useAutomationStatus';
import { agoText, automationsApi, errorText, type AutomationStatus, type ReminderSummary } from '../../../utils/automations';
import { QUIET_BUTTON } from './parts';
import { ConvertedNote, MakeRuleButton } from './ReminderRuleActions';

export function RemindersGroup({ status, now }: { status: AutomationStatus | null; now: number }) {
  const openSettings = useAppStore((s) => s.openSettings);
  const [problem, setProblem] = useState<string | null>(null);
  const reminders = status?.reminders ?? [];

  const setEnabled = (r: ReminderSummary, enabled: boolean) => {
    automationsApi
      .setReminders({ op: 'set_enabled', id: r.id, enabled })
      .then(() => {
        setProblem(null);
        void refreshAutomationStatus();
      })
      .catch((e: unknown) => setProblem(errorText(e)));
  };

  return (
    <div className="space-y-2">
      {reminders.length === 0 ? (
        <p className="rounded-lg bg-glass/30 px-3.5 py-3 text-[12.5px] leading-relaxed text-textSecondary">
          No reminders yet. Add one in Reminders, or type /remind in chat.
        </p>
      ) : (
        <div className="divide-y divide-borderSubtle/50 rounded-lg bg-glass/30">
          {reminders.map((r) => (
            <div key={r.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3.5 py-2.5">
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13px] leading-relaxed text-textPrimary">{r.name}</p>
                <p className="mt-0.5 truncate text-[11.5px] text-textMuted">
                  {[
                    r.schedule,
                    r.chat,
                    r.last_fired_ms === null ? 'Not posted yet' : `Last posted ${agoText(r.last_fired_ms, now)}`,
                  ].join(' · ')}
                </p>
              </div>
              {r.converted ? (
                <ConvertedNote reminderId={r.id} compact />
              ) : (
                <div className="flex items-center gap-3">
                  <MakeRuleButton reminderId={r.id} />
                  <Toggle
                    enabled={r.enabled}
                    onChange={() => setEnabled(r, !r.enabled)}
                    ariaLabel={r.enabled ? `Turn off ${r.name}` : `Turn on ${r.name}`}
                  />
                </div>
              )}
            </div>
          ))}
        </div>
      )}
      {problem && (
        <p role="alert" className="text-[12px] leading-relaxed text-error">
          {problem}
        </p>
      )}
      <div>
        <button type="button" onClick={() => openSettings('Chat', 'reminders')} className={QUIET_BUTTON}>
          Edit in Reminders
        </button>
      </div>
    </div>
  );
}
