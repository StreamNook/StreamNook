import { useEffect, useState } from 'react';

/**
 * A `m:ss` display ticking down to a deadline Rust sent as unix ms, so a
 * viewer who opens a card mid-window sees the true time left. Null when there
 * is no deadline: the card then shows Rust's badge instead. Reaching zero only
 * stops the display; the card's phase still changes when Rust says so.
 */
export function useDeadlineClock(deadlineMs: number | null | undefined): string | null {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (deadlineMs == null) return;
    // Read the clock at once too: `now` may be from long before this deadline.
    const first = setTimeout(() => setNow(Date.now()), 0);
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      clearTimeout(first);
      clearInterval(timer);
    };
  }, [deadlineMs]);

  if (deadlineMs == null) return null;
  const seconds = Math.max(0, Math.ceil((deadlineMs - now) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
