// "Open the Discord Status editor" from anywhere (a settings search hit, the
// command palette). The Integrations tab may not be mounted yet when asked, so
// the request waits here until the tab takes it.

/** The search/palette section id of the editor's rows. */
export const DISCORD_EDITOR_SECTION = 'discord-status-editor';

export interface EditorRequest {
  /** The row to bring into view, by its title. */
  row?: string;
}

let pending: EditorRequest | null = null;
const listeners = new Set<() => void>();

export function requestDiscordEditor(row?: string): void {
  pending = { row };
  listeners.forEach((l) => l());
}

/** The request waiting, taken (so it opens the editor once). */
export function takeDiscordEditorRequest(): EditorRequest | null {
  const request = pending;
  pending = null;
  return request;
}

export function subscribeDiscordEditorRequest(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
