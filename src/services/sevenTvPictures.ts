// 7TV profile pictures, as the webview draws them.
//
// Rust resolves them (services/seventv_cosmetics_resolver.rs): which picture a
// person wears rides the chatter cosmetics batch and cache, and Rust reduces it
// to one still and one animated rendition per scale. What stays here is
// presentation: choosing a rendition for a box size and a motion budget, and
// gathering the ids one render asks for into a single IPC call, because a list
// mounts dozens of avatars in the same frame. Answers are memoized per window
// for the resolver's own lifetime, so a re-render never asks again.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useSyncExternalStore } from 'react';

export interface SevenTvPictureImage {
  url: string;
  scale: number;
}

/** ProfilePicture in seventv_cosmetics_resolver.rs. */
export interface SevenTvPicture {
  id: string;
  /** First frames, smallest scale first. */
  still: SevenTvPictureImage[];
  /** Moving renditions, smallest scale first; empty for a still picture. */
  animated: SevenTvPictureImage[];
}

/** 7TV's 1x rendition is 32 px square. */
const BASE_PX = 32;

export interface PictureChoice {
  /** The box the picture fills, in CSS px. */
  px: number;
  /** Whether this surface may draw it moving right now. */
  animate: boolean;
  /** Device pixel ratio; the smallest rendition that stays sharp wins. */
  dpr?: number;
}

/**
 * The URL to draw for one surface: the animated rendition when the surface may
 * move and the picture does, otherwise the still. Picks the smallest scale that
 * covers the box at this pixel ratio (decoding a 4x animation for a 32 px row
 * is the cost this exists to avoid), or the largest there is. Null when the
 * picture has nothing to draw for the request, so the caller falls back.
 */
export function pickSevenTvPictureUrl(
  picture: SevenTvPicture | null | undefined,
  { px, animate, dpr = 1 }: PictureChoice,
): string | null {
  if (!picture) return null;
  const list = animate && picture.animated.length > 0 ? picture.animated : picture.still;
  if (list.length === 0) return null;
  const want = Math.max(1, Math.ceil((px * Math.max(dpr, 1)) / BASE_PX));
  return (list.find((image) => image.scale >= want) ?? list[list.length - 1]).url;
}

/**
 * The cosmetics id 7TV knows a streamer row by, or null where 7TV has no such
 * platform (TikTok) or the row's id is not the account id 7TV keys on. Twitch
 * rows are bare ids, Kick rows carry the broadcaster's user id (the space Kick
 * chatters are keyed in), YouTube rows the channel's `UC...` id (7TV's GOOGLE).
 */
export function sevenTvIdForStream(stream: { provider?: string | null; user_id?: string | null }): string | null {
  const id = stream.user_id ?? '';
  switch (stream.provider ?? 'twitch') {
    case 'twitch':
      return /^\d+$/.test(id) ? id : null;
    case 'kick':
      return /^\d+$/.test(id) ? `kick:${id}` : null;
    case 'youtube':
      return /^UC[\w-]{22}$/.test(id) ? `youtube:${id}` : null;
    default:
      return null;
  }
}

export const isAnimatedPicture = (picture: SevenTvPicture | null | undefined): boolean =>
  !!picture && picture.animated.length > 0;

/** Whether `url` is one of this picture's renditions, still or moving. */
export const isPictureRendition = (
  picture: SevenTvPicture | null | undefined,
  url: string | null | undefined,
): boolean =>
  !!picture && !!url && (picture.still.some((i) => i.url === url) || picture.animated.some((i) => i.url === url));

/**
 * How much a surface may move by its own nature:
 * - `animate`: the moving picture (one person, a focal surface: profile card,
 *   pinned card, channel header);
 * - `still`: the first frame only (dense lists: chat rows, viewers);
 * - `hover`: the first frame until the pointer is over it (sidebar, tiles).
 */
export type SevenTvMotion = 'animate' | 'still' | 'hover';

/** The viewer's "Profile pictures" choice: the 7TV picture when someone set
 *  one, or always the platform's own picture. */
export type ProfilePictureSource = 'seventv' | 'twitch';

/** Rust's CosmeticsSettings default, for every install. */
export const DEFAULT_PROFILE_PICTURE_SOURCE: ProfilePictureSource = 'seventv';

/** The saved source, read the way Rust reads it: only 'twitch' turns 7TV
 *  pictures off; unset or anything unknown is the 7TV default. */
export const profilePictureSource = (setting: string | null | undefined): ProfilePictureSource =>
  setting === 'twitch' ? 'twitch' : 'seventv';

/** Whether 7TV can know this cosmetics id: a bare Twitch id, `kick:<id>` or
 *  `youtube:UC...`. Twin of Rust's `has_seventv_platform`, so an id Rust would
 *  drop (TikTok, a login) is never sent, and never re-asked as a failure. */
export function isSevenTvCosmeticsId(id: string): boolean {
  const digits = (s: string) => /^\d{1,20}$/.test(s);
  if (id.startsWith('kick:')) return digits(id.slice(5));
  if (id.startsWith('youtube:')) return /^UC[\w-]{22}$/.test(id.slice(8));
  return digits(id);
}

/** The id to look a 7TV picture up by under this source: none at all on the
 *  platform source, so a surface asks 7TV nothing, and none for an id 7TV
 *  cannot know. With `ask` false nothing is asked, so any id that another
 *  lookup primed (the settings sample) is read as it is. */
export function sevenTvLookupId(
  source: ProfilePictureSource,
  id: string | null | undefined,
  ask = true,
): string | null {
  if (source !== 'seventv' || !id) return null;
  return !ask || isSevenTvCosmeticsId(id) ? id : null;
}

/** The viewer's "Animation" choice for 7TV profile pictures. */
export type ProfilePictureAnimation = 'always' | 'hover' | 'never';

/** Rust's CosmeticsSettings default, for every install. */
export const DEFAULT_PROFILE_PICTURE_ANIMATION: ProfilePictureAnimation = 'always';

/** What a picture does on one surface, before hover and visibility:
 *  `moving`, `hover` (moves while the pointer is over it) or `still`. */
export type PicturePlan = 'moving' | 'hover' | 'still';

/**
 * The viewer's setting applied to one surface, and nothing else: neither the
 * app's Motion setting nor the system's reduce-motion request moves it, so
 * the choice made here is the one that shows. Always (the default, and the
 * reading of an unset or unknown value) moves every picture whatever the
 * surface. On hover lets the surface decide: focal surfaces move, lists move
 * under the pointer, chat rows stay still. Never draws every picture still.
 */
export function picturePlan(
  setting: ProfilePictureAnimation | string | null | undefined,
  surface: SevenTvMotion,
): PicturePlan {
  if (setting === 'never') return 'still';
  if (setting !== 'hover') return 'moving';
  if (surface === 'animate') return 'moving';
  return surface === 'hover' ? 'hover' : 'still';
}

/**
 * Whether the animated rendition is drawn right now. A `moving` picture
 * pauses (draws its still) while it is out of view or the window is hidden,
 * so a list of moving pictures only costs what is seen; one in view always
 * plays at its own frame rate. `onScreen` means actually in view: on screen,
 * drawn, and not under an overlay or the full-screen player
 * (utils/pictureVisibility).
 */
export function pictureMovesNow(
  plan: PicturePlan,
  { hovered, onScreen, windowHidden }: { hovered: boolean; onScreen: boolean; windowHidden: boolean },
): boolean {
  if (plan === 'moving') return onScreen && !windowHidden;
  if (plan === 'hover') return hovered && !windowHidden;
  return false;
}

// ---- Per-window memo -------------------------------------------------------------

/** The resolver's USER_TTL: a picture changed on 7TV shows within this. */
const ANSWER_TTL_MS = 5 * 60_000;
/** The resolver's HARD_FAIL_TTL: an id whose lookup failed is asked again. */
const RETRY_MS = 30_000;
/** One frame: everything a render mounts lands in the same call. */
const FLUSH_DELAY_MS = 16;
/** The command resolves at most this many ids per call. */
const CALL_MAX = 200;
const MEMO_MAX = 2000;

interface Entry {
  picture: SevenTvPicture | null;
  at: number;
  ttl: number;
}

const answers = new Map<string, Entry>();
const listeners = new Map<string, Set<() => void>>();
const pending = new Set<string>();
const inFlight = new Set<string>();
let flushTimer: ReturnType<typeof setTimeout> | null = null;

const isFresh = (entry: Entry | undefined): boolean => !!entry && Date.now() - entry.at < entry.ttl;

function store(id: string, entry: Entry): void {
  const before = answers.get(id);
  answers.delete(id);
  answers.set(id, entry);
  if (answers.size > MEMO_MAX) {
    const oldest = answers.keys().next().value;
    if (oldest !== undefined && !listeners.has(oldest)) answers.delete(oldest);
  }
  if (before?.picture?.id !== entry.picture?.id) {
    listeners.get(id)?.forEach((notify) => notify());
  }
}

async function flush(): Promise<void> {
  flushTimer = null;
  const ids = [...pending];
  pending.clear();
  for (let i = 0; i < ids.length; i += CALL_MAX) {
    const batch = ids.slice(i, i + CALL_MAX);
    batch.forEach((id) => inFlight.add(id));
    let result: Record<string, SevenTvPicture | null> = {};
    try {
      result = (await invoke<Record<string, SevenTvPicture | null>>('seventv_profile_pictures', { ids: batch })) ?? {};
    } catch {
      // Treated like an id the resolver could not answer: retried soon.
    }
    const now = Date.now();
    for (const id of batch) {
      inFlight.delete(id);
      if (id in result) {
        store(id, { picture: result[id] ?? null, at: now, ttl: ANSWER_TTL_MS });
      } else {
        // Keep whatever we last drew rather than blinking back to the
        // platform picture over a failed refresh.
        store(id, { picture: answers.get(id)?.picture ?? null, at: now, ttl: RETRY_MS });
      }
    }
  }
}

function request(id: string): void {
  listenForLatePictures();
  if (pending.has(id) || inFlight.has(id) || isFresh(answers.get(id))) return;
  pending.add(id);
  flushTimer ??= setTimeout(() => void flush(), FLUSH_DELAY_MS);
}

/** Rust's `PICTURES_EVENT`: pictures whose images arrived after the cosmetics
 *  answer that named them (chat answers paints and badges first). */
const PICTURES_EVENT = 'seventv-profile-pictures';
let listening = false;

function listenForLatePictures(): void {
  if (listening) return;
  listening = true;
  const stopped = () => {
    listening = false;
  };
  try {
    listen<Record<string, SevenTvPicture>>(PICTURES_EVENT, ({ payload }) => {
      const now = Date.now();
      for (const [id, picture] of Object.entries(payload ?? {})) {
        store(id, { picture, at: now, ttl: ANSWER_TTL_MS });
      }
    }).catch(stopped);
  } catch {
    stopped();
  }
}

/** Record an answer another lookup already carried (a chatter's cosmetics, the
 *  profile card's inventory), so no surface asks for it again. A picture whose
 *  images were still on their way arrives later through `PICTURES_EVENT`. */
export function primeSevenTvPicture(id: string, picture: SevenTvPicture | null | undefined): void {
  if (!id) return;
  listenForLatePictures();
  store(id, { picture: picture ?? null, at: Date.now(), ttl: ANSWER_TTL_MS });
}

/** The 7TV picture for a cosmetics id (bare Twitch id, `kick:<id>`,
 *  `youtube:UC...`), or null while unknown or when there is none. Pass null to
 *  ask for nothing. `ask: false` only reads what another lookup primed, for a
 *  surface whose people are already being resolved (chat rows). */
export function useSevenTvPicture(id: string | null | undefined, ask = true): SevenTvPicture | null {
  const subscribe = useCallback(
    (notify: () => void) => {
      if (!id) return () => {};
      let set = listeners.get(id);
      if (!set) listeners.set(id, (set = new Set()));
      set.add(notify);
      if (ask) request(id);
      return () => {
        set.delete(notify);
        if (set.size === 0) listeners.delete(id);
      };
    },
    [id, ask],
  );
  const snapshot = useCallback(() => (id ? (answers.get(id)?.picture ?? null) : null), [id]);
  return useSyncExternalStore(subscribe, snapshot);
}

/** Test seam: forget every answer. */
export function resetSevenTvPicturesForTests(): void {
  answers.clear();
  listeners.clear();
  pending.clear();
  inFlight.clear();
  if (flushTimer) clearTimeout(flushTimer);
  flushTimer = null;
}
