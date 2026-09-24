// snippetStore: the command palette's snippets, as this window presents them.
//
//   customSnippets  user-authored entries layered on top of the built-in
//                   library. Same shape as built-in; their ids are prefixed
//                   `custom.` so they can't collide.
//   favoriteIds     snippet ids (built-in OR custom) the user has starred.
//                   Favorites float to the top of the Snippets section.
//   aliases         snippet id -> user-typed shortcut. Typing the alias in the
//                   palette boosts that snippet above normal title matches.
//                   Case-insensitive.
//
// The data lives in settings (`settings.snippets`, models/settings.rs), so a
// backup carries it and every window reads one copy. A write patches that one
// key; Rust announces it, and other windows re-read it. This window never sends
// the rest of settings, so it cannot revert what another window saved. The
// three localStorage keys it used to live in are imported once, then removed.

import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';
import { patchSettings, SENDER_ID, SETTINGS_UPDATED_EVENT, type SettingsUpdatedPayload } from '../utils/settingsBroadcast';
import { BUILTIN_SNIPPET_IDS, type Snippet } from '../utils/commandPaletteCopypastas';
import type { Settings, SnippetSettings } from '../types';

const LEGACY_CUSTOM = 'streamnook.snippets.custom.v1';
const LEGACY_FAVORITES = 'streamnook.snippets.favorites.v1';
const LEGACY_ALIASES = 'streamnook.snippets.aliases.v1';

export type CustomSnippet = Snippet & { custom: true };

interface SnippetStoreState {
  customSnippets: CustomSnippet[];
  favoriteIds: Set<string>;
  aliases: Map<string, string>;

  addCustomSnippet: (input: { title: string; category: Snippet['category']; content: string; keywords?: string }) => string;
  updateCustomSnippet: (id: string, patch: Partial<Pick<Snippet, 'title' | 'content' | 'category' | 'keywords'>>) => void;
  removeCustomSnippet: (id: string) => void;

  toggleFavorite: (id: string) => void;
  isFavorite: (id: string) => boolean;

  setAlias: (id: string, alias: string) => void;
  clearAlias: (id: string) => void;
  getAlias: (id: string) => string | undefined;
}

type SnippetView = Pick<SnippetStoreState, 'customSnippets' | 'favoriteIds' | 'aliases'>;

// ---------- Settings <-> view ------------------------------------------------

function isStored(s: unknown): s is SnippetSettings['custom'][number] {
  return (
    !!s &&
    typeof s === 'object' &&
    typeof (s as { id?: unknown }).id === 'string' &&
    typeof (s as { title?: unknown }).title === 'string' &&
    typeof (s as { content?: unknown }).content === 'string'
  );
}

/** The store's view of the stored section. Tolerant: a hand-edited or partial
 *  section yields what is readable, never a throw. */
export function viewOf(stored: Partial<SnippetSettings> | null | undefined): SnippetView {
  const custom = Array.isArray(stored?.custom) ? stored.custom.filter(isStored) : [];
  const favorites = Array.isArray(stored?.favorites) ? stored.favorites.filter((x) => typeof x === 'string') : [];
  const aliases = new Map<string, string>();
  const rawAliases = stored?.aliases && typeof stored.aliases === 'object' ? stored.aliases : {};
  for (const [id, alias] of Object.entries(rawAliases)) {
    if (typeof alias === 'string' && alias.trim()) aliases.set(id, alias.trim().toLowerCase());
  }
  return {
    customSnippets: custom.map((s) => ({
      id: s.id,
      title: s.title,
      category: s.category as Snippet['category'],
      content: s.content,
      keywords: s.keywords || undefined,
      custom: true as const,
    })),
    favoriteIds: new Set(favorites),
    aliases,
  };
}

/** The stored section for a view. */
export function storedOf(view: SnippetView): SnippetSettings {
  return {
    custom: view.customSnippets.map(({ id, title, category, content, keywords }) =>
      keywords ? { id, title, category, content, keywords } : { id, title, category, content },
    ),
    favorites: Array.from(view.favoriteIds),
    aliases: Object.fromEntries(view.aliases),
  };
}

function isEmpty(s: SnippetSettings): boolean {
  return s.custom.length === 0 && s.favorites.length === 0 && Object.keys(s.aliases).length === 0;
}

function viewOfState(state: SnippetView): SnippetView {
  return { customSnippets: state.customSnippets, favoriteIds: state.favoriteIds, aliases: state.aliases };
}

/** Show `view` now and save it. */
function commit(view: SnippetView): SnippetView {
  patchSettings({ snippets: storedOf(view) }).catch((err) => {
    Logger.warn('[snippetStore] save failed:', err);
  });
  return view;
}

// ---------- Zustand store ---------------------------------------------------

export const useSnippetStore = create<SnippetStoreState>((set, get) => ({
  customSnippets: [],
  favoriteIds: new Set(),
  aliases: new Map(),

  addCustomSnippet: (input) => {
    // Custom ids are namespaced + random-suffixed so two snippets with the
    // same title don't collide and so they sort distinct from built-ins in
    // any id-keyed lookup.
    const id = `custom.${slugify(input.title)}.${Math.random().toString(36).slice(2, 6)}`;
    const snippet: CustomSnippet = {
      id,
      title: input.title.trim() || 'Untitled',
      category: input.category,
      content: input.content,
      keywords: input.keywords?.trim() || undefined,
      custom: true,
    };
    set((state) => commit({ ...viewOfState(state), customSnippets: [...state.customSnippets, snippet] }));
    return id;
  },

  updateCustomSnippet: (id, patch) => {
    set((state) => {
      const next = state.customSnippets.map((s) =>
        s.id === id
          ? {
              ...s,
              title: patch.title?.trim() || s.title,
              content: patch.content ?? s.content,
              category: patch.category ?? s.category,
              keywords: patch.keywords === undefined ? s.keywords : patch.keywords.trim() || undefined,
            }
          : s,
      );
      return commit({ ...viewOfState(state), customSnippets: next });
    });
  },

  removeCustomSnippet: (id) => {
    set((state) => {
      const customSnippets = state.customSnippets.filter((s) => s.id !== id);
      // Tidy up favorite + alias rows that point at the deleted snippet so
      // they don't accumulate as ghost entries in storage.
      const favoriteIds = new Set(state.favoriteIds);
      favoriteIds.delete(id);
      const aliases = new Map(state.aliases);
      aliases.delete(id);
      return commit({ customSnippets, favoriteIds, aliases });
    });
  },

  toggleFavorite: (id) => {
    set((state) => {
      const favoriteIds = new Set(state.favoriteIds);
      if (favoriteIds.has(id)) favoriteIds.delete(id);
      else favoriteIds.add(id);
      return commit({ ...viewOfState(state), favoriteIds });
    });
  },

  isFavorite: (id) => get().favoriteIds.has(id),

  setAlias: (id, alias) => {
    const normalized = alias.trim().toLowerCase();
    set((state) => {
      const aliases = new Map(state.aliases);
      if (!normalized) aliases.delete(id);
      else aliases.set(id, normalized);
      return commit({ ...viewOfState(state), aliases });
    });
  },

  clearAlias: (id) => {
    set((state) => {
      const aliases = new Map(state.aliases);
      aliases.delete(id);
      return commit({ ...viewOfState(state), aliases });
    });
  },

  getAlias: (id) => get().aliases.get(id),
}));

// ---------- Loading and cross-window sync ----------------------------------

function readLegacy(): SnippetSettings | null {
  try {
    const custom = JSON.parse(localStorage.getItem(LEGACY_CUSTOM) || '[]');
    const favorites = JSON.parse(localStorage.getItem(LEGACY_FAVORITES) || '[]');
    const aliases = JSON.parse(localStorage.getItem(LEGACY_ALIASES) || '{}');
    const stored = storedOf(viewOf({ custom, favorites, aliases }));
    return isEmpty(stored) ? null : stored;
  } catch (err) {
    Logger.warn('[snippetStore] legacy snippets unreadable:', err);
    return null;
  }
}

function dropLegacy(): void {
  try {
    localStorage.removeItem(LEGACY_CUSTOM);
    localStorage.removeItem(LEGACY_FAVORITES);
    localStorage.removeItem(LEGACY_ALIASES);
  } catch {
    /* storage unavailable: nothing to drop */
  }
}

/** Read the stored snippets into this window's store. On the first run after
 *  the move to settings, snippets still in localStorage are saved there first
 *  (only when settings hold none, so a second window cannot double them). */
export async function reloadSnippetStore(): Promise<void> {
  try {
    const settings = await invoke<Settings>('load_settings');
    let stored = storedOf(viewOf(settings.snippets));
    const legacy = readLegacy();
    if (legacy && isEmpty(stored)) {
      await patchSettings({ snippets: legacy });
      stored = legacy;
    }
    if (legacy) dropLegacy();
    useSnippetStore.setState(viewOf(stored));
  } catch (err) {
    Logger.warn('[snippetStore] load failed:', err);
  }
}

let started = false;

/** Load once per window and follow saves made in other windows. */
export function startSnippetSync(): void {
  if (started) return;
  started = true;
  void reloadSnippetStore();
  listen<SettingsUpdatedPayload>(SETTINGS_UPDATED_EVENT, (event) => {
    if (event.payload?.source === SENDER_ID) return;
    if (event.payload?.keys?.includes('snippets')) void reloadSnippetStore();
  }).catch((err) => Logger.warn('[snippetStore] sync listener failed:', err));
}

// ---------- Helpers --------------------------------------------------------

function slugify(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32);
}

/** Helper for places that need the "is this ID built-in?" check without
 *  reaching into the copypasta module — re-exported here so the snippet
 *  settings page has one import surface for everything snippet-related. */
export { BUILTIN_SNIPPET_IDS };
