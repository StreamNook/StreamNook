// "What changes" at the top of a rule made from a reminder: every field whose
// behavior differs now, in Rust's words, until the person says they have read it.
import { dismissWhatChanges, useWhatChangesStore } from '../../../utils/reminderRules';
import { Block, QUIET_BUTTON } from './parts';

export function WhatChanges({ ruleId }: { ruleId: string }) {
  const changes = useWhatChangesStore((s) => s.byRule[ruleId]);
  if (!ruleId || !changes || changes.length === 0) return null;
  return (
    <Block title="What changes">
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
    </Block>
  );
}
