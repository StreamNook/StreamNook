/** Discord's elapsed timer text for a start in Unix seconds, as its activity
 *  card draws it: "0:13" and "12:04" under an hour, "1:12:44" from an hour on
 *  (the leading unit is never padded). */
export function elapsedText(startSecs: number, nowMs: number): string {
  const total = Math.max(0, Math.floor(nowMs / 1000) - startSecs);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}
