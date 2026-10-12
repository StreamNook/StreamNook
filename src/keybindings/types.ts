// Keybinding system — shared types.
//
// One registry of "bindable commands" feeds three surfaces: the global key
// dispatcher (useKeybindings), the Ctrl+K command palette (shortcut hints +
// searchable actions), and the Keybindings settings tab (recorder UI). Keeping
// a single source of truth means the palette and the settings tab can never
// drift out of sync.

/** Where a binding is allowed to fire. Mirrors Chatterino's category/context
 *  idea: the same combo can mean different things depending on focus + app
 *  state. Only `global` and `player` are dispatched by the engine today; the
 *  rest are documentation-only contexts whose keys are owned by their own
 *  components (chat input, MultiChat tabs, modals). */
export type KeybindContext =
  | 'global' // anywhere in the app, when not typing and no modal is open
  | 'player' // only while a stream/VOD is playing
  | 'chatInput' // the chat compose field (owned by ChatWidget)
  | 'chatPane' // the chat message list (reserved for a later phase)
  | 'multiView' // MultiChat / MultiNook (owned by those windows)
  | 'popup'; // command palette / modal dialogs

/** Top-level grouping shown as section headers in the settings tab. */
export type KeybindCategory =
  | 'Application'
  | 'Navigation'
  | 'Player'
  | 'Chat'
  | 'Moderation'
  | 'Multi-view';

export interface BindableCommand {
  /** Stable id. Where it matches a command-palette item id (e.g. 'qa.openDrops',
   *  'cs.toggleTheatre', 'player.mute'), the palette row inherits this command's
   *  shortcut hint automatically. */
  id: string;
  label: string;
  description?: string;
  category: KeybindCategory;
  context: KeybindContext;
  /** Canonical default chord strings, e.g. ['Ctrl+,'] or ['Space', 'K'].
   *  Empty means unbound by default. */
  defaultBindings: string[];
  /** Extra search terms for the settings tab and palette. */
  keywords?: string;
  /** When true the binding is shown for discoverability but is owned by a
   *  component or the OS: it cannot be rebound or cleared, and the engine never
   *  dispatches it (the owning code already handles the key). */
  reserved?: boolean;
  /** Runs the action. Omitted for reserved documentation-only entries. */
  run?: () => void | Promise<void>;
  /** Whether the command is currently runnable (e.g. only while watching).
   *  Used both by the dispatcher and to gray out rows in the settings tab. */
  isAvailable?: () => boolean;
  /** When true, holding the key auto-repeats the action (volume, seek). When
   *  false/omitted, key auto-repeat is swallowed so a held key fires once —
   *  important for toggles like mute/play-pause/theatre. */
  repeatable?: boolean;
  /** For a key the page answers too (Esc closes whatever is open): the
   *  command runs only after the page has had the key, and only when nothing on
   *  the way marked it handled (preventDefault or stopPropagation). The key is
   *  never swallowed for it. */
  deferred?: boolean;
}
