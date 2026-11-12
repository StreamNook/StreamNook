// The small notices a person can switch off one by one (Settings >
// Notifications > Small notices). The ids, their words and their switches are
// Rust's (src-tauri/src/services/notice_types.json, `get_notice_types`); a
// call site names its notice with `addToast(..., { notice })`, and a notice
// whose switch `live_notifications.notice_<id>` is off does not show. Errors
// and warnings never carry one, so they can never be hidden.
import { useEffect, useState } from 'react';
import { invokeList } from './invokeList';
import type { LiveNotificationSettings } from '../types';

/** Every notice id; the list in notice_types.json is tested to match. */
export const NOTICE_IDS = [
  'welcome_back',
  'drop_claimed',
  'copied',
  'quality_changed',
  'stream_restarted',
  'vod_resumed',
] as const;
export type NoticeId = (typeof NOTICE_IDS)[number];

/** Mirrors Rust `notice_types::NoticeType`. */
export interface NoticeType {
  id: NoticeId;
  title: string;
  detail: string;
  /** The dotted settings path its switch writes (`live_notifications.notice_copied`). */
  setting: string;
}

/** The `live_notifications` key a notice's switch writes. */
export const noticeKey = (id: NoticeId): keyof LiveNotificationSettings =>
  `notice_${id}` as keyof LiveNotificationSettings;

/** Whether the person switched this notice off (every switch starts on). */
export function noticeHidden(prefs: Partial<LiveNotificationSettings> | undefined, id: NoticeId | undefined): boolean {
  return id !== undefined && prefs?.[noticeKey(id)] === false;
}

let cached: NoticeType[] | null = null;
let pending: Promise<NoticeType[]> | null = null;

/** The notices this build shows, read once per run, retried while Rust does
 *  not answer (`invokeList`); the next caller tries again after a total miss. */
export function loadNoticeTypes(): Promise<NoticeType[]> {
  if (cached) return Promise.resolve(cached);
  pending ??= invokeList<NoticeType>('get_notice_types').then((list) => {
    if (list.length > 0) cached = list;
    else pending = null;
    return list;
  });
  return pending;
}

/** The list once read; null until then. */
export function useNoticeTypes(): NoticeType[] | null {
  const [list, setList] = useState<NoticeType[] | null>(cached);
  useEffect(() => {
    // Always asks: the list can land between the first render and this
    // effect, and returning early then left the section empty for the run.
    let live = true;
    void loadNoticeTypes().then((l) => {
      if (live) setList(l);
    });
    return () => {
      live = false;
    };
  }, []);
  return list;
}
