// The local views a viewer turned on with /localmod, /localsub and
// /localascii: this app shows only matching rows in that channel, nobody else
// is affected. Which rows match is Rust's call: the rule engine stamps the
// built-in ids `local:mod`, `local:sub` and `local:ascii` into
// metadata.filter_ids on every message. Session-only on purpose: a forgotten
// view should not hide chat after a restart.

import { create } from 'zustand';

export type LocalView = 'mod' | 'sub' | 'ascii';

export const LOCAL_VIEW_FILTER_ID: Record<LocalView, string> = {
  mod: 'local:mod',
  sub: 'local:sub',
  ascii: 'local:ascii',
};

interface LocalChatViewState {
  /** channel key (bare lowercase login) -> views in force */
  views: Record<string, LocalView[]>;
  setView: (channel: string, view: LocalView, on: boolean) => void;
}

const EMPTY: LocalView[] = [];

export const useLocalChatViewStore = create<LocalChatViewState>((set) => ({
  views: {},
  setView: (channel, view, on) =>
    set((s) => {
      const key = channel.toLowerCase();
      const now = s.views[key] ?? EMPTY;
      const next = on ? Array.from(new Set([...now, view])) : now.filter((v) => v !== view);
      return { views: { ...s.views, [key]: next } };
    }),
}));

/** The filter ids a row must carry to show in `channel`, [] when no view is on. */
export function useLocalViewFilterIds(channel: string | null | undefined): string[] {
  const views = useLocalChatViewStore((s) => (channel ? s.views[channel.toLowerCase()] : undefined) ?? EMPTY);
  return views.map((v) => LOCAL_VIEW_FILTER_ID[v]);
}

/** Whether a row passes the local views. Rows the rule engine never judged
 *  (system notices) always show. */
export function passesLocalViews(
  row: { metadata?: { filter_ids?: string[]; rules_evaluated?: boolean } } | string,
  required: string[],
): boolean {
  if (required.length === 0 || typeof row === 'string') return true;
  if (!row.metadata?.rules_evaluated) return true;
  const ids = row.metadata.filter_ids ?? [];
  return required.every((id) => ids.includes(id));
}
