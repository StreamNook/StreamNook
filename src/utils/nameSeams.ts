/** A run-together name split where its words meet ("PrincessDonutBrown" ->
 *  "Princess", "Donut", "Brown"; "RoadToTwitchCon26" ends "Con", "26"), so a
 *  narrow label can wrap between words instead of mid-word. A lowercase letter
 *  before a capital, or a letter before a digit, is a seam; runs of capitals
 *  ("XIV") and a digit before letters ("7TV") stay whole. Joined back
 *  together, the pieces are the name unchanged. */
export function nameSeams(name: string): string[] {
  const pieces: string[] = [];
  let start = 0;
  for (let i = 1; i < name.length; i++) {
    const a = name[i - 1];
    const b = name[i];
    const lowerToUpper = a >= 'a' && a <= 'z' && b >= 'A' && b <= 'Z';
    const letterToDigit = /[A-Za-z]/.test(a) && b >= '0' && b <= '9';
    if (lowerToUpper || letterToDigit) {
      pieces.push(name.slice(start, i));
      start = i;
    }
  }
  pieces.push(name.slice(start));
  return pieces;
}
