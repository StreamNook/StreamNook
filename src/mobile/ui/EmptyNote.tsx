// A list with nothing to show: a line saying why, and a quieter line under it
// saying what to do. The why reads first (brighter and a little heavier); the
// hint is a step down. One component so every empty list on the phone speaks
// with the same voice instead of each screen rolling its own muted copy.
import React from 'react';

export const EmptyNote: React.FC<{
  text: string;
  /** What to do about it. null when the reason already says it. */
  hint?: string | null;
}> = ({ text, hint = 'Pull down to refresh.' }) => (
  <div className="flex justify-center px-6 py-20 text-center">
    <div className="flex max-w-[300px] flex-col gap-1.5">
      <p className="text-[15px] font-medium leading-5 text-textSecondary">{text}</p>
      {hint && <p className="text-[13px] leading-[18px] text-textMuted">{hint}</p>}
    </div>
  </div>
);
