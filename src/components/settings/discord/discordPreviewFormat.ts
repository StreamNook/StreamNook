/** Discord's elapsed timer text for a start in Unix seconds: "12:04" under an
 *  hour, "01:12:44" from an hour on. */
export function elapsedText(startSecs: number, nowMs: number): string {
  const total = Math.max(0, Math.floor(nowMs / 1000) - startSecs);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${pad(h)}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}
