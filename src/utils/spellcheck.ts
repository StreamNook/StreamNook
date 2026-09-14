// Main-thread side of chat spell checking.
//
// Two jobs: own the worker's lifecycle, and decide which words the worker is
// even allowed to see. The second half is the part that matters — the webview's
// own spell checker underlines every emote name and every login, which is
// exactly the noise this replaces.
//
// Word filtering happens in three layers:
//   1. Text shape      — tokenizeForSpellcheck (pure, unit-tested)
//   2. Chat vocabulary — this file: channel emotes, chatters, custom dictionary
//   3. English         — the worker
//
// Nothing here touches the network. The dictionary ships with the app.

import { getChannelEmotes } from '../stores/chatConnectionStore';
import { getEmoteLookup } from '../services/emoteService';
import { useChatUserStore } from '../stores/chatUserStore';
import { useAppStore } from '../stores/AppStore';
import { tokenizeForSpellcheck } from './chatInputWord';
import { Logger } from './logger';
import type { SpellRequest, SpellResponse } from '../workers/spellcheck.worker';

/** Tear the worker down after this long with no traffic. Each window is its own
 *  JS realm, so a user with the main window plus a few MultiChat popouts would
 *  otherwise hold that many parsed copies of the dictionary indefinitely. */
const IDLE_TEARDOWN_MS = 5 * 60 * 1000;

/** Ceiling on how long the context menu can sit on "Checking spelling…". */
const SUGGEST_TIMEOUT_MS = 1000;

/** What the caller knows about where the text is being typed. */
export interface SpellContext {
  /** Emote-cache key for the composer's channel (see `emoteCacheKey`), or null
   *  for surfaces with no channel of their own, like Whispers. */
  emoteKey: string | null;
}

let worker: Worker | null = null;
let nextId = 1;
let idleTimer: ReturnType<typeof setTimeout> | null = null;
const pending = new Map<number, (response: SpellResponse) => void>();

function scheduleIdleTeardown(): void {
  if (idleTimer) clearTimeout(idleTimer);
  idleTimer = setTimeout(() => {
    // Never pull the worker out from under an in-flight request.
    if (pending.size > 0) {
      scheduleIdleTeardown();
      return;
    }
    worker?.terminate();
    worker = null;
    idleTimer = null;
  }, IDLE_TEARDOWN_MS);
}

function ensureWorker(): Worker {
  if (!worker) {
    worker = new Worker(new URL('../workers/spellcheck.worker.ts', import.meta.url), {
      type: 'module',
    });
    worker.onmessage = (event: MessageEvent<SpellResponse>) => {
      const resolve = pending.get(event.data.id);
      if (!resolve) return;
      pending.delete(event.data.id);
      resolve(event.data);
    };
    worker.onerror = (event) => {
      Logger.warn('[Spellcheck] worker error:', event.message);
    };
  }
  scheduleIdleTeardown();
  return worker;
}

/** A plain `Omit` over a union collapses it to the keys all members share, which
 *  would lose `words` and `word`. Distributing keeps each variant intact. */
type WithoutId<T> = T extends unknown ? Omit<T, 'id'> : never;

/** Send one request and wait for its reply. Resolves to null if the worker
 *  errors or the deadline passes, so callers can degrade to "no result"
 *  instead of throwing into a render path. */
function request(
  message: WithoutId<SpellRequest>,
  timeoutMs: number,
): Promise<SpellResponse | null> {
  return new Promise((resolve) => {
    const id = nextId++;
    let settled = false;

    const finish = (value: SpellResponse | null) => {
      if (settled) return;
      settled = true;
      pending.delete(id);
      resolve(value);
    };

    const timer = setTimeout(() => finish(null), timeoutMs);

    pending.set(id, (response) => {
      clearTimeout(timer);
      if (response.type === 'error') {
        Logger.warn('[Spellcheck] worker reported:', response.message);
        finish(null);
        return;
      }
      finish(response);
    });

    try {
      ensureWorker().postMessage({ ...message, id } as SpellRequest);
    } catch (err) {
      clearTimeout(timer);
      Logger.warn('[Spellcheck] failed to post to worker:', err);
      finish(null);
    }
  });
}

/** Kick off dictionary loading without waiting for it. Called when a composer
 *  takes focus, so the first right-click already has a warm engine. */
export function warmSpellcheck(): void {
  void request({ type: 'warm' }, 30_000);
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
  const response = await request({ type: 'check', words: unique }, SUGGEST_TIMEOUT_MS * 5);
  if (!response || response.type !== 'check') return [];

  const misspelled = new Set(response.misspelled);
  return tokens
    .filter((t) => misspelled.has(t.word))
    .map((t) => [t.start, t.end] as [number, number]);
}

export interface SpellVerdict {
  /** False only when the word is definitely misspelled. A worker timeout
   *  reports `true` so a hiccup never puts corrections on a good word. */
  correct: boolean;
  /** Corrections, best first. Empty for a correct word, and also possible for a
   *  misspelled one the dictionary can't get close to. */
  suggestions: string[];
}

/** Ask whether one word is spelled correctly, and what it should be if not. */
export async function suggestWord(word: string): Promise<SpellVerdict> {
  const response = await request({ type: 'suggest', word }, SUGGEST_TIMEOUT_MS);
  return response && response.type === 'suggest'
    ? { correct: response.correct, suggestions: response.suggestions }
    : { correct: true, suggestions: [] };
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
