// Chat spell checking, as the composers see it.
//
// English is Rust's job (services/spellcheck.rs): one dictionary shared by the
// main window and every MultiChat popout. This file decides which words Rust is
// even allowed to see. The webview's own spell checker underlines every emote
// name and every login, which is exactly the noise this replaces.
//
// Word filtering happens in three layers:
//   1. Text shape      - tokenizeForSpellcheck (pure, unit-tested)
//   2. Chat vocabulary - this file: channel emotes, chatters, custom dictionary
//   3. English         - Rust
//
// Layer 2 stays here because the emote sets and the chatter registry it reads
// are still held in this window's stores, and the context menu needs its
// answer synchronously. Nothing here touches the network.

import { invoke } from '@tauri-apps/api/core';
import { getChannelEmotes } from '../stores/chatConnectionStore';
import { getEmoteLookup } from '../services/emoteService';
import { useChatUserStore } from '../stores/chatUserStore';
import { useAppStore } from '../stores/AppStore';
import { tokenizeForSpellcheck } from './chatInputWord';
import { Logger } from './logger';

/** Ceiling on how long the context menu can sit on "Checking spelling...". */
const SUGGEST_TIMEOUT_MS = 1000;

/** What the caller knows about where the text is being typed. */
export interface SpellContext {
  /** Emote-cache key for the composer's channel (see `emoteCacheKey`), or null
   *  for surfaces with no channel of their own, like Whispers. */
  emoteKey: string | null;
}

/** One spelling command, or null if it fails or misses its deadline, so callers
 *  degrade to "no result" instead of throwing into a render path. */
function ask<T>(command: string, args: Record<string, unknown>, timeoutMs: number): Promise<T | null> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), timeoutMs);
    invoke<T>(command, args).then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err) => {
        clearTimeout(timer);
        Logger.warn(`[Spellcheck] ${command} failed:`, err);
        resolve(null);
      },
    );
  });
}

/** Kick off dictionary loading without waiting for it. Called when a composer
 *  takes focus, so the first right-click already has a warm engine. */
export function warmSpellcheck(): void {
  void ask('spell_warm', {}, 30_000);
}

/** The user's own additions, lowercased for comparison. */
function customWords(): Set<string> {
  const words = useAppStore.getState().settings?.chat_input?.spellcheck_custom_words;
  return new Set((words ?? []).map((w) => w.toLowerCase()));
}

/** True when this word is chat vocabulary rather than English: an emote in the
 *  current channel, someone in chat, or a word the user has added. `custom`
 *  lets batch callers build the user-additions set once instead of per word. */
export function isKnownChatToken(
  word: string,
  ctx: SpellContext,
  custom: Set<string> = customWords(),
): boolean {
  const lower = word.toLowerCase();

  if (custom.has(lower)) return true;

  if (useChatUserStore.getState().getUserByUsername(lower)) return true;

  if (ctx.emoteKey) {
    const emotes = getChannelEmotes(ctx.emoteKey);
    if (emotes) {
      // Per-set name index, invalidated by set identity (sets are replaced
      // wholesale on /refresh and channel switch, never mutated).
      if (getEmoteLookup(emotes).lowerNames.has(lower)) return true;
    }
  }

  return false;
}

/** Ranges of `text` that are misspelled, as [start, end) pairs. */
export async function checkText(
  text: string,
  ctx: SpellContext,
): Promise<Array<[number, number]>> {
  const custom = customWords();
  const tokens = tokenizeForSpellcheck(text).filter((t) => !isKnownChatToken(t.word, ctx, custom));
  if (tokens.length === 0) return [];

  // One round trip for the whole composer, deduped — "the the the" asks once.
  const unique = [...new Set(tokens.map((t) => t.word))];
  const response = await ask<string[]>('spell_check', { words: unique }, SUGGEST_TIMEOUT_MS * 5);
  if (!response) return [];

  const misspelled = new Set(response);
  return tokens
    .filter((t) => misspelled.has(t.word))
    .map((t) => [t.start, t.end] as [number, number]);
}

export interface SpellVerdict {
  /** False only when the word is definitely misspelled. A timeout
   *  reports `true` so a hiccup never puts corrections on a good word. */
  correct: boolean;
  /** Corrections, best first. Empty for a correct word, and also possible for a
   *  misspelled one the dictionary can't get close to. */
  suggestions: string[];
}

/** Ask whether one word is spelled correctly, and what it should be if not. */
export async function suggestWord(word: string): Promise<SpellVerdict> {
  const response = await ask<SpellVerdict>('spell_suggest', { word }, SUGGEST_TIMEOUT_MS);
  return response ?? { correct: true, suggestions: [] };
}

/** Teach the checker a word. Persists through settings, which also broadcasts
 *  to any open MultiChat popouts so they stop flagging it too. */
export async function addToCustomDictionary(word: string): Promise<void> {
  const lower = word.toLowerCase();
  const state = useAppStore.getState();
  const settings = state.settings;
  const existing = settings.chat_input?.spellcheck_custom_words ?? [];

  if (existing.some((w) => w.toLowerCase() === lower)) return;

  await state.updateSettings({
    ...settings,
    chat_input: {
      ...settings.chat_input,
      spellcheck_custom_words: [...existing, lower],
    },
  });
}
