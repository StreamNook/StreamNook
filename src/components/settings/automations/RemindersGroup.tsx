// The Reminders group on the Automations page: every reminder in one card, one
// row each with its schedule, chat, when it last posted, its ⋯ menu and its
// switch. Reminders stay reminders: "Edit in Reminders" opens their own section,
// and "Make it a rule" is the only way one joins the rules. The page draws the
// section around this (the settings search reads sections from
// AutomationsSettings.tsx only).
import { createElement, useState } from 'react';
import { CalendarClock, ExternalLink, MessageSquareText, Timer, Undo2, Workflow } from 'lucide-react';
import { useAppStore } from '../../../stores/AppStore';
import { Toggle } from '../../ui/Toggle';
import { IS_MOBILE } from '../../../utils/platform';
import { refreshAutomationStatus, useAutomationStatusStore } from '../../../hooks/useAutomationStatus';
import { agoText, automationsApi, errorText, type AutomationStatus, type ReminderSummary } from '../../../utils/automations';
import { convertReminder, restoreReminder, ruleMadeFrom } from '../../../utils/reminderRules';
import { CARD, ErrorLine, GHOST_BUTTON, HINT, META, MenuButton, QuietTag, type MenuItem } from './parts';

const LEAD = IS_MOBILE ? 'w-6' : 'w-[18px]';

/** The mark for how a reminder fires, read from its schedule words. */
function markOf(schedule: string) {
  if (/^every/i.test(schedule)) return Timer;
  if (/say|word|keyword/i.test(schedule)) return MessageSquareText;
  return CalendarClock;
}

/** "in #channel", or "in the current channel". */
function whereText(chat: string): string {
  return chat.startsWith('#') ? `in ${chat}` : `in the ${chat.charAt(0).toLowerCase()}${chat.slice(1)}`;
}

function ReminderRow({ r, now, onProblem }: { r: ReminderSummary; now: number; onProblem: (text: string | null) => void }) {
  const openSettings = useAppStore((s) => s.openSettings);
  const ruleId = useAutomationStatusStore((s) => ruleMadeFrom(s.status, r.id));
  const run = (work: Promise<unknown>) => {
    onProblem(null);
    work.then(() => void refreshAutomationStatus()).catch((e: unknown) => onProblem(errorText(e)));
  };
  const edit: MenuItem = {
    id: 'edit',
    label: 'Edit in Reminders',
    icon: ExternalLink,
    onSelect: () => openSettings('Chat', 'reminders'),
  };
  const menu: MenuItem[] = r.converted
    ? [{ id: 'back', label: 'Turn back on', help: 'Posts as a reminder again', icon: Undo2, onSelect: () => run(restoreReminder(r.id)) }, edit]
    : [
        {
          id: 'rule',
          label: 'Make it a rule',
          help: 'A new rule in Test; this reminder turns off',
          icon: Workflow,
          onSelect: () => {
            onProblem(null);
            convertReminder(r.id)
              .then((answer) => openSettings('Automations', undefined, `rule:${answer.rule_id}`))
              .catch((e: unknown) => onProblem(errorText(e)));
          },
        },
        edit,
      ];

  return (
    <div className="flex items-start gap-3">
      <span className={`${LEAD} flex shrink-0 justify-center pt-[2px] ${r.enabled ? 'text-accent-on-surface' : 'text-textMuted'}`}>
        {/* An element, not a JSX tag: the mark is looked up per row. */}
        {createElement(markOf(r.schedule), { size: IS_MOBILE ? 20 : 17, 'aria-hidden': true })}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className={`truncate ${IS_MOBILE ? 'text-[15px]' : 'text-[14px]'} font-medium ${r.enabled || r.converted ? 'text-textPrimary' : 'text-textSecondary'}`}
          >
            {r.name}
          </span>
          {r.converted && <QuietTag>Now a rule</QuietTag>}
        </div>
        <p className={`mt-0.5 ${HINT}`}>
          {r.schedule} {whereText(r.chat)}
        </p>
        {!r.converted && (
          <p className={`mt-1 ${META}`}>
            {r.last_fired_ms === null ? 'Not posted yet' : `Last posted ${agoText(r.last_fired_ms, now)}`}
          </p>
        )}
      </div>
      <span className="flex shrink-0 items-center gap-1.5">
        {r.converted && ruleId && (
          <button
            type="button"
            onClick={() => openSettings('Automations', undefined, `rule:${ruleId}`)}
            className={GHOST_BUTTON}
          >
            Open rule
          </button>
        )}
        <MenuButton label={`${r.name} options`} items={menu} />
        {/* A converted reminder has no switch; the gap keeps its ⋯ in the column the others share. */}
        {r.converted && <span aria-hidden className="w-11 shrink-0" />}
        {!r.converted && (
          <Toggle
            enabled={r.enabled}
            onChange={() => run(automationsApi.setReminders({ op: 'set_enabled', id: r.id, enabled: !r.enabled }))}
            ariaLabel={r.enabled ? `Turn off ${r.name}` : `Turn on ${r.name}`}
          />
        )}
      </span>
    </div>
  );
}

export function RemindersGroup({ status, now }: { status: AutomationStatus | null; now: number }) {
  const [problem, setProblem] = useState<string | null>(null);
  const reminders = status?.reminders ?? [];

  if (reminders.length === 0) {
    return (
      <div className={`${CARD} px-4 py-6 text-center`}>
        <p className="text-[13px] text-textSecondary">No reminders yet. Add one with New reminder, or type /remind in chat.</p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className={`${CARD} automation-rows`}>
        {reminders.map((r) => (
          <ReminderRow key={r.id} r={r} now={now} onProblem={setProblem} />
        ))}
      </div>
      {problem && <ErrorLine className={IS_MOBILE ? 'px-4' : 'px-1'}>{problem}</ErrorLine>}
    </div>
  );
}
