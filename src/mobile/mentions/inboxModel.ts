// Pure rules the phone's inbox draws with: row keys, which ids a row marks as
// seen, paging, the counts' revision, and a whisper row's words. Kept apart
// from the components so they are tested without a page.
import type {
  InboxFilterId,
  InboxItem,
  InboxMentionItem,
  InboxViewPayload,
  MentionEntry,
  WhisperRowMeta,
} from '../../types';

/** Who a face shows and which mark it wears. */
export interface FaceInfo {
  name: string;
  color: string | null;
  kind: 'mention' | 'reply' | 'word' | 'whisper';
}

/** The chips shown before Rust's first answer. Whispers join only when Rust
 *  lists them (a Twitch account is signed in). */
export const DEFAULT_CHIPS: { id: InboxFilterId; unread: number }[] = [
  { id: 'all', unread: 0 },
  { id: 'mentions', unread: 0 },
  { id: 'highlights', unread: 0 },
];

export function chipsOf(view: Pick<InboxViewPayload, 'chips'> | null): { id: InboxFilterId; unread: number }[] {
  return view?.chips?.length ? view.chips : DEFAULT_CHIPS;
}

/** The ids a row marks read once the inbox closes after showing it. A whisper
 *  conversation is read only by opening it, so it marks nothing. */
export function seenIdsOf(item: InboxItem): string[] {
  return item.kind === 'mention' ? [item.entry.id, ...item.folded.map((f) => f.id)] : [];
}

export function mentionItems(rows: InboxItem[]): InboxMentionItem[] {
  return rows.filter((r): r is InboxMentionItem => r.kind === 'mention');
}

/** The next page after the loaded rows: a row already shown is not added twice. */
export function mergePage(cur: InboxViewPayload, page: InboxViewPayload): InboxViewPayload {
  const have = new Set(cur.rows.map((r) => r.row_id));
  return { ...page, rows: [...cur.rows, ...page.rows.filter((r) => !have.has(r.row_id))] };
}

export interface CountsState {
  unreadTotal: number;
  countsRev: number;
}

/** The counts after an answer with revision `rev`, or null when that answer is
 *  older than what is shown (a view read that left before a change and came
 *  back after its event). */
export function nextCounts(cur: CountsState, total: number, rev: number): CountsState | null {
  if (rev < cur.countsRev) return null;
  if (rev === cur.countsRev && total === cur.unreadTotal) return null;
  return { unreadTotal: total, countsRev: rev };
}

/** The person a conversation is with, never empty. */
export function whisperName(m: Pick<WhisperRowMeta, 'user_name' | 'user_login'>): string {
  return m.user_name || m.user_login || 'Someone';
}

/** A whisper row's second line when it has no emotes: the newest message,
 *  yours marked as yours. */
export function whisperPreview(m: Pick<WhisperRowMeta, 'last_text' | 'last_is_sent'>): string {
  if (!m.last_text) return '';
  return m.last_is_sent ? `You: ${m.last_text}` : m.last_text;
}

export function faceOfEntry(e: MentionEntry): FaceInfo {
  return { name: e.sender_name || e.sender_login || '?', color: e.sender_color ?? null, kind: e.kind };
}

export function faceOfWhisper(m: WhisperRowMeta): FaceInfo {
  return { name: whisperName(m), color: m.color, kind: 'whisper' };
}
