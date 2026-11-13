// Reminders and the rules they become: "Make it a rule", "Turn back on", and
// the "What changes" list a rule made from a reminder shows until it is
// dismissed. Rust does the work (commands/account_automations.rs); this wraps
// it and keeps what the editor needs to show afterwards.

import { create } from 'zustand';
import { refreshAutomationStatus } from '../hooks/useAutomationStatus';
import { automationsApi, type AutomationStatus, type ReminderChange, type ReminderConversion } from './automations';

/** Said once on the Reminders group and its section. */
export const REMINDERS_OWN_SWITCHES = 'Reminders follow their own switches, not the status above.';

interface WhatChangesState {
  /** "What changes" by the id of the rule made from a reminder. */
  byRule: Record<string, ReminderChange[]>;
}

export const useWhatChangesStore = create<WhatChangesState>(() => ({ byRule: {} }));

export function dismissWhatChanges(ruleId: string): void {
  useWhatChangesStore.setState((s) => {
    const rest = { ...s.byRule };
    delete rest[ruleId];
    return { byRule: rest };
  });
}

/** "Make it a rule": Rust builds the rule in Test, switches the reminder off and lists it as converted.
 *  The "What changes" lines are kept for the editor that opens next; the status is read again. */
export async function convertReminder(id: string): Promise<ReminderConversion> {
  const answer = await automationsApi.convertReminder(id);
  useWhatChangesStore.setState((s) => ({ byRule: { ...s.byRule, [answer.rule_id]: answer.changes } }));
  void refreshAutomationStatus();
  return answer;
}

/** "Turn back on": the reminder runs again, its rule is switched off. */
export async function restoreReminder(id: string): Promise<void> {
  await automationsApi.restoreReminder(id);
  void refreshAutomationStatus();
}

/** The id of the rule a converted reminder became, while that rule still exists. */
export function ruleMadeFrom(status: Pick<AutomationStatus, 'rules'> | null, reminderId: string): string | null {
  return status?.rules.find((r) => r.rule?.origin === `reminder:${reminderId}`)?.id ?? null;
}
