// Pane focus bridge for MultiChat custom splits: the window registers how to
// move keyboard focus between panes; the keybinding engine calls through.

export interface PaneFocusController {
  /** True when a split layout with more than one pane is showing. */
  isAvailable(): boolean;
  focusNext(): void;
  focusPrev(): void;
}

let current: PaneFocusController | null = null;

export function registerPaneFocusController(controller: PaneFocusController | null): void {
  current = controller;
}

export function getPaneFocusController(): PaneFocusController | null {
  return current;
}
