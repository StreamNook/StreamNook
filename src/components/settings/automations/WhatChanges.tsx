// "What changes" at the top of a rule made from a reminder: every field whose
// behavior differs now, in Rust's words, until the person says they have read it.
import { Info } from 'lucide-react';
import { dismissWhatChanges, useWhatChangesStore } from '../../../utils/reminderRules';
import { QUIET_BUTTON } from './parts';

export function WhatChanges({ ruleId }: { ruleId: string }) {
  const changes = useWhatChangesStore((s) => s.byRule[ruleId]);
  if (!ruleId || !changes || changes.length === 0) return null;
  return (
    <section className="space-y-2 rounded-lg bg-accent/[0.08] p-3.5">
      <div className="flex items-center gap-2">
        <Info size={14} className="text-accent" />
        <h4 className="text-[13px] font-semibold text-textPrimary">What changes</h4>
      </div>
      <p className="text-[12px] leading-relaxed text-textMuted">
        This rule was made from a reminder and starts in Test. These work differently now.
      </p>
      <ul className="space-y-1.5">
        {changes.map((change, i) => (
          <li key={`${change.field}-${i}`} className="text-[12.5px] leading-relaxed text-textSecondary">
            {change.words}
          </li>
        ))}
      </ul>
      <div>
        <button type="button" onClick={() => dismissWhatChanges(ruleId)} className={QUIET_BUTTON}>
          Got it
        </button>
      </div>
    </section>
  );
}
