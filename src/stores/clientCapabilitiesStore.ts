import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { IS_MOBILE } from '../utils/platform';
import { Logger } from '../utils/logger';

/**
 * The desktop-only surfaces this build has, as Rust reports them
 * (`services/client_capabilities.rs`, `get_client_capabilities`). Settings
 * rows for a surface the build lacks are simply not drawn; search and the
 * changed-settings review hide the same rows in Rust.
 */
export interface ClientCapabilities {
  /** Another platform's chat for the same streamer merged into this one. */
  combined_chat: boolean;
  /** Several chats kept open beside the stream. */
  chat_dock: boolean;
  /** Highlight and mention marks on the chat scrollbar. */
  scrollbar_marks: boolean;
  /** Words taught to the chat spellchecker (a phone spells with its keyboard). */
  spell_dictionary: boolean;
}

const DESKTOP: ClientCapabilities = {
  combined_chat: true,
  chat_dock: true,
  scrollbar_marks: true,
  spell_dictionary: true,
};

const NONE: ClientCapabilities = {
  combined_chat: false,
  chat_dock: false,
  scrollbar_marks: false,
  spell_dictionary: false,
};

/** Until Rust answers: a phone starts with none, so a desktop-only row never
 *  flashes in, and everything else (desktop, tests, a plain browser) starts
 *  with the desktop answer, so desktop draws exactly as it always has. */
const BEFORE_ANSWER = IS_MOBILE ? NONE : DESKTOP;

export const useClientCapabilities = create<ClientCapabilities>(() => ({ ...BEFORE_ANSWER }));

const inTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in (window as unknown as Record<string, unknown>);

let loaded = false;

/** Read the answer once per window. Safe to call from every boot path. */
export function loadClientCapabilities(): void {
  if (loaded || !inTauri()) return;
  loaded = true;
  invoke<ClientCapabilities>('get_client_capabilities')
    .then((caps) => useClientCapabilities.setState({ ...NONE, ...caps }))
    .catch((e) => {
      loaded = false;
      Logger.warn('[capabilities] could not read the client capabilities:', e);
    });
}

loadClientCapabilities();
