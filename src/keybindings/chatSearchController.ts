// Chat search control bridge (Ctrl+F).
//
// Every mounted ChatWidget registers a controller; the keybinding engine asks
// for the one that should receive the shortcut: the pane under the pointer,
// else the main-window widget, else the first registered. Mirrors the
// chatModController pattern so the engine never reaches into chat internals.

export interface ChatSearchController {
  /** The widget the user is looking at (hovered, or the sole main widget). */
  isActive(): boolean;
  /** True for the main-window widget (no channel override). */
  isMain(): boolean;
  /** Open (or focus) the search bar. */
  openSearch(): void;
  /** Close the search bar if open. Returns true when it was open. */
  closeSearch(): boolean;
}

const controllers = new Set<ChatSearchController>();

export function registerChatSearchController(controller: ChatSearchController): () => void {
  controllers.add(controller);
  return () => {
    controllers.delete(controller);
  };
}

export function getChatSearchController(): ChatSearchController | null {
  if (controllers.size === 0) return null;
  let main: ChatSearchController | null = null;
  let first: ChatSearchController | null = null;
  for (const c of controllers) {
    if (c.isActive()) return c;
    if (!main && c.isMain()) main = c;
    if (!first) first = c;
  }
  return main ?? first;
}
