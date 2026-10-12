// A key pressed while an input method is composing (Japanese, Chinese, Korean
// and the like) belongs to that composition until it commits: Enter confirms
// the candidate, the arrows walk the candidates, Escape cancels it. A text
// field's own shortcuts (send on Enter, list navigation) must leave those keys
// alone. `keyCode` 229 is how some engines report a composing key when
// `isComposing` is not set on it.

interface KeyLike {
  isComposing?: boolean;
  keyCode?: number;
}

/** Whether a key event (a React event or a DOM one) is part of an IME
 *  composition. */
export function isImeComposing(e: KeyLike | { nativeEvent: KeyLike }): boolean {
  const key = 'nativeEvent' in e ? e.nativeEvent : e;
  return !!key.isComposing || key.keyCode === 229;
}
