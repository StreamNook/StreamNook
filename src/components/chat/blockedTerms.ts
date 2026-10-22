// A channel's AutoMod blocked terms, shared by the desktop Tools menu sheet
// and the phone's Manage channel page. Rust lists every page of them, sorts
// them newest first and writes their words (services/twitch_service.rs); this
// file only holds the list while a sheet is open and runs the two writes.
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface BlockedTermRow {
  id: string;
  text: string;
  /** "Added 3 days ago"; empty when Twitch gave no time. */
  added: string;
  /** "Expires in 3d" for AutoMod's temporary terms; null for terms that stay. */
  expires: string | null;
  created_at_ms: number;
  expires_at_ms: number | null;
}

/** Twitch takes a phrase of 2 to 500 characters. */
export const BLOCKED_TERM_MIN = 2;
export const BLOCKED_TERM_MAX = 500;

/** Why Twitch would refuse this phrase for its length, or null when it fits. */
export function blockedTermProblem(text: string): string | null {
  const length = [...text.trim()].length;
  if (length < BLOCKED_TERM_MIN) return `Use at least ${BLOCKED_TERM_MIN} characters.`;
  if (length > BLOCKED_TERM_MAX) return `Keep it to ${BLOCKED_TERM_MAX} characters or fewer.`;
  return null;
}

/** The rows whose phrase contains `query`, ignoring case; all of them for an empty query. */
export function filterBlockedTerms(rows: BlockedTermRow[], query: string): BlockedTermRow[] {
  const wanted = query.trim().toLowerCase();
  if (!wanted) return rows;
  return rows.filter((r) => r.text.toLowerCase().includes(wanted));
}

const words = (err: unknown) => (typeof err === 'string' && err ? err : 'That did not go through');

/**
 * The list, read once when the sheet mounts. `busy` names the write in flight
 * ('add' or a row's id); `note` is the last write's refusal, in Twitch's words.
 */
export function useBlockedTerms(broadcasterId: string) {
  const [rows, setRows] = useState<BlockedTermRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    if (!broadcasterId) return;
    let alive = true;
    invoke<BlockedTermRow[]>('list_blocked_terms', { broadcasterId })
      .then((next) => alive && setRows(next))
      .catch((err) => alive && setError(words(err)));
    return () => {
      alive = false;
    };
  }, [broadcasterId, attempt]);

  const retry = useCallback(() => {
    setError(null);
    setRows(null);
    setAttempt((n) => n + 1);
  }, []);

  /** Blocks a phrase and reads the list again so it shows in Rust's words. */
  const add = useCallback(
    async (text: string): Promise<boolean> => {
      const phrase = text.trim();
      const problem = blockedTermProblem(phrase);
      if (problem) {
        setNote(problem);
        return false;
      }
      setBusy('add');
      setNote(null);
      try {
        await invoke('add_blocked_term', { broadcasterId, text: phrase });
      } catch (err) {
        setNote(words(err));
        setBusy(null);
        return false;
      }
      try {
        setRows(await invoke<BlockedTermRow[]>('list_blocked_terms', { broadcasterId }));
        setError(null);
      } catch {
        setNote(`Blocked “${phrase}”. The list did not refresh; reopen it to see it.`);
      } finally {
        setBusy(null);
      }
      return true;
    },
    [broadcasterId],
  );

  const remove = useCallback(
    async (row: BlockedTermRow) => {
      setBusy(row.id);
      setNote(null);
      try {
        await invoke('remove_blocked_term_by_id', { broadcasterId, id: row.id });
        setRows((cur) => cur && cur.filter((r) => r.id !== row.id));
      } catch (err) {
        setNote(words(err));
      } finally {
        setBusy(null);
      }
    },
    [broadcasterId],
  );

  return { rows, error, busy, note, retry, add, remove };
}
