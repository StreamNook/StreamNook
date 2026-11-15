// What a category link's id turned out to be. A link opens its screen at once
// with an empty name; the screen asks for the id it shows, and the answer is
// kept here by id so every screen that shows the id (and the store's copies in
// history) can name itself from it.
//
// Rust has no outer time bound on the lookup, so this module owns the one
// wait: a 6 s timer stores `failed`, and an answer that lands later still
// replaces it when nothing newer was asked.

import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchCategory } from '../types';
import { readCategoryAnswer } from './inboundLinkOpen';

export type LinkedCategory = {
  state: 'loading' | 'found' | 'failed' | 'missing';
  category?: TwitchCategory;
  /** Store time in ms; written for `found` only. */
  at?: number;
};

const ANSWER_LIMIT_MS = 6000;
const STALE_MS = 60 * 60 * 1000;
const MAX_RECORDS = 64;

type Flight = {
  token: number;
  promise: Promise<LinkedCategory>;
  timer: ReturnType<typeof setTimeout>;
};

// Insertion order is the LRU order, oldest first. Only a write or a resolve call
// reorders it; a read never does, so rendering cannot change what it reads.
const records = new Map<string, LinkedCategory>();
// Kept apart from `records` so a background re-ask never has to turn a `found`
// record into `loading`.
const inFlight = new Map<string, Flight>();
// The newest try for each id. A late answer may write only for the newest try.
const latestToken = new Map<string, number>();
let tokenCounter = 0;

const subscribers = new Set<() => void>();
const lateListeners = new Set<(id: string, category: TwitchCategory) => void>();

/** The store subscription behind `useLinkedCategory`; also how tests watch writes. */
export function subscribeLinkedCategories(listener: () => void): () => void {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}

function notify(): void {
  for (const listener of [...subscribers]) listener();
}

// A record nothing is waiting on may go; the one just stored never does.
function evictable(id: string, justStored: string | undefined): boolean {
  return id !== justStored && records.get(id)?.state !== 'loading' && !inFlight.has(id);
}

// Runs after every write and every settle: a single eviction on the 65th write
// would leave the map over its cap for good when the only candidates were
// still in flight at that moment.
function tidy(justStored: string | undefined): void {
  while (records.size > MAX_RECORDS) {
    const oldest = [...records.keys()].find((id) => evictable(id, justStored));
    if (oldest === undefined) break;
    records.delete(oldest);
    latestToken.delete(oldest);
  }
  // With no record and no entry no answer for the id may write, so its token
  // changes nothing (a forgotten record leaves its token behind).
  for (const id of [...latestToken.keys()]) {
    if (!records.has(id) && !inFlight.has(id)) latestToken.delete(id);
  }
}

function write(id: string, record: LinkedCategory): void {
  records.delete(id);
  records.set(id, record);
  tidy(id);
  notify();
}

function categoryOf(id: string, answer: { name: string; box_art_url: string }): TwitchCategory {
  return { id, name: answer.name, box_art_url: answer.box_art_url };
}

async function ask(id: string): Promise<ReturnType<typeof readCategoryAnswer>> {
  try {
    return readCategoryAnswer(await invoke<unknown>('resolve_category_link', { id }));
  } catch {
    return null;
  }
}

function start(id: string, background: boolean): Promise<LinkedCategory> {
  const token = ++tokenCounter;
  latestToken.set(id, token);
  const holds = () => inFlight.get(id)?.token === token;
  const current = (): LinkedCategory => records.get(id) ?? { state: 'failed' };

  let finish: (record: LinkedCategory) => void = () => {};
  const promise = new Promise<LinkedCategory>((resolve) => {
    finish = resolve;
  });

  const timer = setTimeout(() => {
    if (holds()) {
      inFlight.delete(id);
      // A background re-ask that times out writes no record: the old answer stays.
      if (background) tidy(undefined);
      else write(id, { state: 'failed' });
    }
    finish(current());
  }, ANSWER_LIMIT_MS);
  // The entry exists before any callback can run: the timer and the answer are both async.
  inFlight.set(id, { token, promise, timer });

  void ask(id).then((answer) => {
    if (holds()) {
      clearTimeout(timer);
      inFlight.delete(id);
      if (answer?.state === 'found') {
        const category = categoryOf(id, answer);
        write(id, { state: 'found', category, at: Date.now() });
      } else if (background) {
        tidy(undefined);
      } else {
        write(id, { state: answer?.state === 'missing' ? 'missing' : 'failed' });
      }
      finish(current());
      return;
    }
    // The timer already stored `failed`. A found or missing answer from the
    // newest try for this id replaces it, like a late badge hit fills a failed
    // sheet; anything else writes nothing.
    if (answer?.state !== 'found' && answer?.state !== 'missing') return;
    if (inFlight.has(id) || records.get(id)?.state !== 'failed' || latestToken.get(id) !== token) {
      return;
    }
    if (answer.state === 'missing') {
      write(id, { state: 'missing' });
      return;
    }
    const category = categoryOf(id, answer);
    write(id, { state: 'found', category, at: Date.now() });
    for (const listener of [...lateListeners]) {
      try {
        listener(id, category);
      } catch {
        // One failing listener must not stop the others.
      }
    }
  });

  return promise;
}

/**
 * The answer for a category id. A `found` or `missing` record returns at once;
 * `loading` joins the call already running; `failed` returns as it is unless
 * `retry` asks again. A `found` record older than an hour also re-asks once in
 * the background and stays `found` meanwhile.
 */
export function resolveLinkedCategory(
  id: string,
  opts?: { retry?: boolean },
): Promise<LinkedCategory> {
  const record = records.get(id);
  if (record) {
    records.delete(id);
    records.set(id, record);
    if (record.state === 'found') {
      if (!inFlight.has(id) && Date.now() - (record.at ?? 0) > STALE_MS) void start(id, true);
      return Promise.resolve(record);
    }
    if (record.state === 'loading') {
      const flight = inFlight.get(id);
      if (flight) return flight.promise;
    } else if (record.state === 'missing' || !opts?.retry) {
      return Promise.resolve(record);
    }
  }
  const loading: LinkedCategory = { state: 'loading' };
  const promise = start(id, false);
  write(id, loading);
  return promise;
}

/** The record for an id, read without touching the LRU order. */
export function getLinkedCategory(id: string): LinkedCategory | undefined {
  return records.get(id);
}

/** The record for the id a screen shows; re-renders the screen when it changes. */
export function useLinkedCategory(id: string | null): LinkedCategory | undefined {
  const readRecord = () => (id ? records.get(id) : undefined);
  return useSyncExternalStore(subscribeLinkedCategories, readRecord, readRecord);
}

/**
 * A new link for an id is a fresh attempt: drop that id's `failed` or `missing`
 * record. A `found` or `loading` record stays, so a resolve in flight is never
 * cancelled by a repeat link (and no new token is issued for it).
 */
export function forgetFailedLinkedCategory(id: string): void {
  const record = records.get(id);
  if (!record || (record.state !== 'failed' && record.state !== 'missing')) return;
  records.delete(id);
  // An answer from a try before this link must not write after it.
  latestToken.set(id, ++tokenCounter);
  notify();
}

/** Called for every `found` that lands after its `failed` was stored. */
export function onLateLinkedCategory(
  listener: (id: string, category: TwitchCategory) => void,
): () => void {
  lateListeners.add(listener);
  return () => {
    lateListeners.delete(listener);
  };
}

/** For tests: forget every record and call; keeps subscribers and late listeners. */
export function resetLinkedCategories(): void {
  for (const flight of inFlight.values()) clearTimeout(flight.timer);
  records.clear();
  inFlight.clear();
  latestToken.clear();
  notify();
}

/** For tests: the size of each map. */
export function linkedCategorySizesForTest(): { records: number; tokens: number; inFlight: number } {
  return { records: records.size, tokens: latestToken.size, inFlight: inFlight.size };
}

/**
 * The nameless copy of `id` filled from a `found` record. Null for another id,
 * a copy that already has a name, no copy, or a record that is not `found`; an
 * answer is a fact about an id, so filling twice is a no-op.
 */
export function categoryAnswerOutcome(
  copy: { id: string; name: string } | null,
  id: string,
  record: LinkedCategory | undefined,
): TwitchCategory | null {
  if (!copy || copy.id !== id || copy.name !== '') return null;
  if (record?.state !== 'found' || !record.category) return null;
  return { ...copy, name: record.category.name, box_art_url: record.category.box_art_url };
}
