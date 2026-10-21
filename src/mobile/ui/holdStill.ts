// HoldWhile's memo comparator: skip the render only while the hold stays on
// both sides, so the render that releases it (or starts it) goes through.
export function holdStill(prev: { hold: boolean }, next: { hold: boolean }): boolean {
  return prev.hold && next.hold;
}
