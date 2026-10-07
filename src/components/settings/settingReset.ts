// Reset arrows for settings rows: an arrow beside a row's title appears only
// while that setting differs from what a fresh install starts with, and
// clicking it puts the default back.
//
// The defaults are Rust's (`get_default_settings`, which is
// `Settings::default()`, phone values on a phone build). Groups the frontend
// owns are absent there, so a reset removes the key and the page's own
// fallback applies, the same as on a fresh install; a row names that fallback
// beside the path so the arrow can tell whether it still matches.
//
// Why this runs in the page: a reset is the same write the row's control
// makes, so it goes through `updateSettings` and gets that path's side effects
// (Discord connect, diagnostics, chat placement) and its cross-window patch.
// Whether to show the arrow is a comparison against the store the row already
// renders from; asking Rust would cost a round trip per change and lag the
// control.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import type { Settings } from '../../types';
import { Logger } from '../../utils/logger';

/** A dotted settings path (`chat_design.font_size`), or the path with the
 *  fallback its row reads when the key is absent (`['chat_input.spellcheck', true]`). */
export type ResetSpec = string | readonly [path: string, fallback: unknown];

let cached: Settings | null = null;
let pending: Promise<Settings | null> | null = null;

function loadDefaults(): Promise<Settings | null> {
  if (cached) return Promise.resolve(cached);
  pending ??= invoke<Settings>('get_default_settings')
    .then((d) => (cached = d))
    .catch((e) => {
      Logger.warn('[Settings] Could not load defaults:', e);
      pending = null;
      return null;
    });
  return pending;
}

type Json = Record<string, unknown>;

function read(root: unknown, path: string): unknown {
  let cur = root;
  for (const part of path.split('.')) {
    if (!cur || typeof cur !== 'object') return undefined;
    cur = (cur as Json)[part];
  }
  return cur;
}

/** A copy of `root` with `path` set to `value`, or removed when `value` is
 *  undefined. Objects along the way are copied, never mutated. */
function write(root: Json, path: string, value: unknown): Json {
  const [head, ...rest] = path.split('.');
  const next = { ...root };
  if (rest.length === 0) {
    if (value === undefined) delete next[head];
    else next[head] = value;
    return next;
  }
  const child = next[head];
  next[head] = write(child && typeof child === 'object' ? (child as Json) : {}, rest.join('.'), value);
  return next;
}

// Arrays compare element-wise and objects key-wise (order-independent): a list
// edited back to its default is a new array with the same contents. Absent and
// null mean the same thing in a settings file.
function same(a: unknown, b: unknown): boolean {
  if (a === b || (a == null && b == null)) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => same(v, b[i]));
  }
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a as object).filter((k) => (a as Json)[k] != null);
    const kb = Object.keys(b as object).filter((k) => (b as Json)[k] != null);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => same((a as Json)[k], (b as Json)[k]));
  }
  return false;
}

const parse = (spec: ResetSpec): [string, unknown] =>
  typeof spec === 'string' ? [spec, undefined] : [spec[0], spec[1]];

/** Whether every setting the specs name still holds its default. */
export function atDefault(settings: Settings, defaults: Settings, specs: ResetSpec[]): boolean {
  return specs.map(parse).every(([path, fallback]) =>
    same(read(settings, path) ?? fallback, read(defaults, path) ?? fallback),
  );
}

/** `settings` with every setting the specs name put back to its default:
 *  Rust's value first, the row's fallback for a key Rust does not hold. Only
 *  a key with neither is removed, so a field a typed Rust struct requires is
 *  never dropped from the save. */
export function withDefaults(settings: Settings, defaults: Settings, specs: ResetSpec[]): Settings {
  let next = settings as unknown as Json;
  for (const [path, fallback] of specs.map(parse)) {
    const value = read(defaults, path) ?? fallback;
    next = write(next, path, value == null ? undefined : structuredClone(value));
  }
  return next as unknown as Settings;
}

/** Returns `resetFor(...specs)`: undefined while every setting it names still
 *  holds its default (so the row shows no arrow), otherwise a function that
 *  restores them all. A row with more than one control names every key it
 *  covers, so they reset as a unit. */
export function useSettingReset() {
  const settings = useAppStore((s) => s.settings);
  const [defaults, setDefaults] = useState<Settings | null>(cached);

  useEffect(() => {
    if (defaults) return;
    let live = true;
    void loadDefaults().then((d) => {
      if (live && d) setDefaults(d);
    });
    return () => {
      live = false;
    };
  }, [defaults]);

  return (...specs: ResetSpec[]): ResetFn | undefined => {
    if (!defaults || atDefault(settings, defaults, specs)) return undefined;
    return () => {
      // The store at click time, not at render: another row may have saved since.
      const { settings: current, updateSettings } = useAppStore.getState();
      const next = withDefaults(current, defaults, specs);
      void updateSettings(next);
      return next;
    };
  };
}

/** Restores a row's settings and returns the settings it saved. */
export type ResetFn = () => Settings;

/** A reset that also runs what the row's control runs after a change (a
 *  runtime switch, a refresh), handed the settings the reset saved. */
export const thenRun = (
  reset: ResetFn | undefined,
  after: (next: Settings) => void,
): ResetFn | undefined =>
  reset &&
  (() => {
    const next = reset();
    after(next);
    return next;
  });
