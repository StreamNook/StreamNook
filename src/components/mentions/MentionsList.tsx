// MultiChat's inbox (its Mentions mode): every mention, reply and highlight
// Rust filed for the signed-in account, folded like the phones', and the
// account's whisper conversations, newest first. Rust owns the rows, their read
// state and the counts; this draws a page of them and sends what the viewer
// does. The main window shows the same inbox in the Dynamic Island.

import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Archive, BellOff, Bell, Check, CheckCheck, Inbox, Trash2, X } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { ProviderLogo } from '../ProviderLogo';
import { PinnedChatter } from '../chat/PinnedChatter';
import { WhisperParts } from '../whispers/WhisperParts';
import { useAppStore } from '../../stores/AppStore';
import { toggleMentionChatMuted, useInboxCounts } from '../../stores/mentionInboxStore';
import { useInboxView } from '../inbox/useInboxView';
import { dropRows, foldedLine, readRows, rowUnread } from '../inbox/islandHistory';
import { isProviderId } from '../../types/providers';
import type { InboxItem, MentionEntry, MentionKind, WhisperRowMeta } from '../../types';
import { agoLabel } from '../../utils/lastLive';
import { Logger } from '../../utils/logger';

type MentionRowView = Extract<InboxItem, { kind: 'mention' }>;
type WhisperRowView = Extract<InboxItem, { kind: 'whisper' }>;

const KIND_WORDS: Record<MentionKind, string> = {
  mention: 'mentioned you',
  reply: 'replied to you',
  word: 'matched a highlight',
};

const timeLabel = (atMs: number, now: number): string =>
  (agoLabel(new Date(atMs).toISOString(), now) ?? '').replace(/ ago$/, '');

const ROW_CLASS =
  'group relative flex cursor-pointer gap-2.5 px-4 py-3 transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover focus-visible:outline-none';
const ACTION_CLASS =
  'flex h-6 w-6 items-center justify-center rounded-md text-textMuted transition-colors hover:bg-surface-active hover:text-textPrimary';

/** A row is a control: Enter or Space opens it. */
const openOnKey = (open: () => void) => (e: React.KeyboardEvent<HTMLDivElement>) => {
  if (e.target !== e.currentTarget) return;
  if (e.key === 'Enter' || e.key === ' ') {
    e.preventDefault();
    open();
  }
};

interface MentionRowProps {
  row: MentionRowView;
  now: number;
  showLogo: boolean;
  muted: boolean;
  onOpen: (entry: MentionEntry) => void;
  onRemove: (row: MentionRowView) => void;
  onToggleMute: (entry: MentionEntry) => void;
}

function MentionRow({ row, now, showLogo, muted, onOpen, onRemove, onToggleMute }: MentionRowProps) {
  const entry = row.entry;
  const read = !row.unread;
  // Rust stores the readable color the chat row shows; drawn as is.
  const color = entry.sender_color || undefined;
  const platform = isProviderId(entry.platform) ? entry.platform : null;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen(entry)}
      onKeyDown={openOnKey(() => onOpen(entry))}
      data-mention-row={entry.id}
      className={ROW_CLASS}
    >
      {/* Unread dot, on the first line's cap height. Read rows keep the
          column so nothing shifts when a row is read. */}
      <span className="flex h-[18px] w-1.5 shrink-0 items-center" aria-hidden>
        {!read && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
      </span>
      {/* A read row dims its content (name, chat, message); the muted words
          around them are already quiet and stay readable. */}
      <div className="min-w-0 flex-1">
        <div className="flex h-[18px] items-center gap-1.5 text-[12.5px] leading-[18px]">
          <span
            className={`min-w-0 max-w-[45%] shrink-[3] truncate font-semibold ${read ? 'opacity-60' : ''}`}
            style={{ color }}
          >
            {entry.sender_name}
          </span>
          <span className="shrink-0 text-textMuted">{KIND_WORDS[entry.kind]} in</span>
          <span className={`flex min-w-[6ch] flex-initial items-center gap-1 text-textSecondary ${read ? 'opacity-60' : ''}`}>
            {showLogo && platform && <ProviderLogo provider={platform} size={12} />}
            <span className="min-w-0 truncate">{entry.channel_login}</span>
          </span>
          {/* Time, swapped for the row's actions on hover. One fixed slot, so
              the line never reflows between the two. */}
          <span className="relative ml-auto flex h-[18px] w-[52px] shrink-0 items-center justify-end pl-1">
            <span className="text-[11px] tabular-nums text-textMuted transition-opacity group-hover:opacity-0 group-focus-within:opacity-0">
              {timeLabel(entry.at_ms, now)}
            </span>
            <span className="absolute inset-y-0 -right-2 flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
              <Tooltip content={muted ? 'Unmute this chat' : 'Mute this chat'} side="top">
                <button
                  type="button"
                  aria-label={muted ? 'Unmute this chat' : 'Mute this chat'}
                  onClick={(e) => { e.stopPropagation(); onToggleMute(entry); }}
                  className={ACTION_CLASS}
                >
                  {muted ? <Bell size={13} /> : <BellOff size={13} />}
                </button>
              </Tooltip>
              <Tooltip content="Remove" side="top">
                <button
                  type="button"
                  aria-label="Remove"
                  onClick={(e) => { e.stopPropagation(); onRemove(row); }}
                  className={ACTION_CLASS}
                >
                  <X size={16} />
                </button>
              </Tooltip>
            </span>
          </span>
        </div>
        {entry.removed ? (
          <p className="mt-1 text-[13px] italic leading-snug text-textMuted">Message deleted by a moderator</p>
        ) : (
          <p className={`mt-1 line-clamp-3 break-words text-[13px] leading-snug text-textPrimary ${read ? 'opacity-60' : ''}`}>
            {entry.text}
          </p>
        )}
        {row.folded.length > 0 && (
          <p className="mt-1 text-[12px] text-textMuted">{foldedLine(entry.kind, row.folded.length + 1)}</p>
        )}
      </div>
    </div>
  );
}

interface WhisperRowProps {
  row: WhisperRowView;
  now: number;
  onOpen: (whisper: WhisperRowMeta) => void;
  onRead: (row: WhisperRowView) => void;
  onArchive: (row: WhisperRowView) => void;
}

function WhisperRow({ row, now, onOpen, onRead, onArchive }: WhisperRowProps) {
  const whisper = row.whisper;
  const read = whisper.unread_count === 0;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen(whisper)}
      onKeyDown={openOnKey(() => onOpen(whisper))}
      data-whisper-row={whisper.key}
      className={ROW_CLASS}
    >
      <span className="flex h-[18px] w-1.5 shrink-0 items-center" aria-hidden>
        {!read && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex h-[18px] items-center gap-1.5 text-[12.5px] leading-[18px]">
          <span className={`flex min-w-0 max-w-[55%] shrink-[3] items-center gap-1 ${read ? 'opacity-60' : ''}`}>
            <PinnedChatter
              provider="twitch"
              userId={whisper.user_id}
              name={whisper.user_name}
              login={whisper.user_login}
              color={whisper.color ?? undefined}
              badges={whisper.badges}
              variant="line"
              nameClassName="min-w-0 truncate font-semibold"
            />
          </span>
          <span className="shrink-0 text-textMuted">{whisper.last_is_sent ? 'you whispered' : 'whispered you'}</span>
          {whisper.unread_count > 1 && (
            <span className="shrink-0 tabular-nums text-textMuted">{whisper.unread_count} new</span>
          )}
          <span className="relative ml-auto flex h-[18px] w-[52px] shrink-0 items-center justify-end pl-1">
            <span className="text-[11px] tabular-nums text-textMuted transition-opacity group-hover:opacity-0 group-focus-within:opacity-0">
              {timeLabel(whisper.last_message_timestamp, now)}
            </span>
            <span className="absolute inset-y-0 -right-2 flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
              {!read && (
                <Tooltip content="Mark read" side="top">
                  <button
                    type="button"
                    aria-label="Mark read"
                    onClick={(e) => { e.stopPropagation(); onRead(row); }}
                    className={ACTION_CLASS}
                  >
                    <Check size={14} />
                  </button>
                </Tooltip>
              )}
              <Tooltip content="Archive" side="top">
                <button
                  type="button"
                  aria-label="Archive"
                  onClick={(e) => { e.stopPropagation(); onArchive(row); }}
                  className={ACTION_CLASS}
                >
                  <Archive size={14} />
                </button>
              </Tooltip>
            </span>
          </span>
        </div>
        <p className={`mt-1 line-clamp-3 break-words text-[13px] leading-snug text-textPrimary ${read ? 'opacity-60' : ''}`}>
          <WhisperParts parts={whisper.last_parts} text={whisper.last_text} />
        </p>
      </div>
    </div>
  );
}

export interface MentionsListProps {
  /** What opening a mention does: open its chat and find the message. */
  onOpen: (entry: MentionEntry) => void;
  /** Controls after the list's own (a close button). */
  headerEnd?: ReactNode;
  /** MultiChat: a slimmer header that fits under the window's own toolbar. */
  compact?: boolean;
}

export function MentionsList({ onOpen, headerEnd, compact = false }: MentionsListProps) {
  const inbox = useInboxView('multichat', true);
  const { patch } = inbox;
  const counts = useInboxCounts();
  const [confirmClear, setConfirmClear] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(tick);
  }, []);

  // This list coming on screen is an inbox opening: Rust syncs whispers with
  // Twitch if a minute has passed since the last time.
  useEffect(() => {
    invoke('inbox_opened').catch((e) => Logger.debug('[Mentions] inbox_opened failed:', e));
  }, []);

  const muted = useAppStore((s) => s.settings.mentions?.muted_channels);
  const mutedSet = useMemo(() => new Set((muted ?? []).map((r) => r.toLowerCase())), [muted]);
  // Rust's MultiChat count: every unread mention and whisper this list shows
  // (mentions seen live included, which the island leaves out).
  const unread = counts?.multichat ?? counts?.island ?? 0;

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 240) inbox.loadOlder();
  };

  const removeMention = useCallback((row: MentionRowView) => {
    patch((rows) => dropRows(rows, [row.row_id]));
    for (const id of [row.entry.id, ...row.folded.map((f) => f.id)]) {
      invoke('remove_mention', { id }).catch((e) => Logger.warn('[Mentions] remove failed:', e));
    }
  }, [patch]);

  const toggleMute = useCallback((entry: MentionEntry) => {
    void toggleMentionChatMuted(entry.room);
  }, []);

  // The conversation opens in the main window's Whispers window, bringing the
  // main window back first when going live closed it.
  const openWhisper = useCallback(async (whisper: WhisperRowMeta) => {
    try {
      const { ensureMainAndEmit } = await import('../../utils/ensureMainWindow');
      await ensureMainAndEmit('start-whisper', {
        id: whisper.user_id,
        login: whisper.user_login,
        display_name: whisper.user_name,
        profile_image_url: whisper.profile_image_url ?? undefined,
      });
    } catch (e) {
      Logger.warn('[Mentions] opening the whisper in the main window failed:', e);
    }
  }, []);

  const readWhisper = useCallback((row: WhisperRowView) => {
    patch((rows) => readRows(rows, [row.row_id]));
    invoke('whisper_mark_read', { key: row.whisper.key }).catch((e) => Logger.warn('[Mentions] whisper mark read failed:', e));
  }, [patch]);

  const archiveWhisper = useCallback((row: WhisperRowView) => {
    patch((rows) => dropRows(rows, [row.row_id]));
    invoke('whisper_archive', { key: row.whisper.key }).catch((e) => Logger.warn('[Mentions] archive failed:', e));
  }, [patch]);

  const markAllRead = () => {
    patch((rows) => readRows(rows, null));
    invoke('mark_mentions_read', {}).catch((e) => Logger.warn('[Mentions] mark read failed:', e));
    invoke('whisper_mark_all_read').catch((e) => Logger.warn('[Mentions] whisper mark all read failed:', e));
  };

  // Mentions are deleted; whispers only leave this list and the island (they
  // stay in the Whispers window and on the phones).
  const clearAll = () => {
    setConfirmClear(false);
    patch(() => []);
    invoke('clear_mention_inbox').catch((e) => Logger.warn('[Mentions] clear failed:', e));
    invoke('whisper_island_clear').catch((e) => Logger.warn('[Mentions] whisper clear failed:', e));
  };

  // A platform mark only where platforms mix in the list.
  const mixed = useMemo(
    () => new Set(inbox.rows.flatMap((r) => (r.kind === 'mention' ? [r.entry.platform] : []))).size > 1,
    [inbox.rows],
  );
  const anyUnread = unread > 0 || inbox.rows.some(rowUnread);

  const iconButton =
    'flex h-8 w-8 items-center justify-center rounded-lg text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary disabled:pointer-events-none disabled:opacity-40';

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className={`flex shrink-0 items-center justify-between gap-3 border-b border-borderSubtle ${compact ? 'h-11 px-3' : 'h-[60px] px-4'}`}>
        <div className="flex min-w-0 items-center gap-2">
          <Inbox size={compact ? 16 : 19} className="shrink-0 text-accent" />
          <span className={`font-semibold text-textPrimary ${compact ? 'text-sm' : 'text-lg'}`}>Inbox</span>
          {unread > 0 && (
            <span className="glass-badge rounded-full px-2 py-0.5 text-xs font-medium tabular-nums text-textPrimary">
              {unread > 99 ? '99+' : unread}
            </span>
          )}
        </div>
        {confirmClear ? (
          <div className="flex shrink-0 items-center gap-2">
            <span className="text-[12.5px] text-textSecondary">Clear mentions and hide whispers?</span>
            <button
              type="button"
              onClick={() => setConfirmClear(false)}
              className="h-7 rounded-lg px-2.5 text-[12.5px] font-medium text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={clearAll}
              className="h-7 rounded-lg bg-red-500/15 px-2.5 text-[12.5px] font-semibold text-red-300 transition-colors hover:bg-red-500/25"
            >
              Clear all
            </button>
          </div>
        ) : (
          <div className={`flex shrink-0 items-center gap-0.5 ${compact ? '-mr-1.5' : '-mr-2.5'}`}>
            <Tooltip content="Mark all read" side="bottom">
              <button type="button" aria-label="Mark all read" onClick={markAllRead} disabled={!anyUnread} className={iconButton}>
                <CheckCheck size={17} />
              </button>
            </Tooltip>
            <Tooltip content="Clear all" side="bottom">
              <button
                type="button"
                aria-label="Clear all"
                onClick={() => setConfirmClear(true)}
                disabled={inbox.rows.length === 0}
                className={iconButton}
              >
                <Trash2 size={16} />
              </button>
            </Tooltip>
            {headerEnd}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto scrollbar-thin" onScroll={onScroll}>
        {inbox.loaded && inbox.rows.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center px-10 text-center">
            <Inbox size={36} className="mb-3 text-accent/40" />
            <span className="text-sm font-medium text-textPrimary">Nothing here yet</span>
            <span className="mt-1 max-w-[30ch] text-[12.5px] leading-relaxed text-textMuted [text-wrap:balance]">
              Mentions, replies and whispers show up here.
            </span>
          </div>
        ) : (
          <div className="py-1">
            {inbox.rows.map((row) =>
              row.kind === 'mention' ? (
                <MentionRow
                  key={row.row_id}
                  row={row}
                  now={now}
                  showLogo={mixed}
                  muted={mutedSet.has(row.entry.room.toLowerCase())}
                  onOpen={onOpen}
                  onRemove={removeMention}
                  onToggleMute={toggleMute}
                />
              ) : (
                <WhisperRow
                  key={row.row_id}
                  row={row}
                  now={now}
                  onOpen={(w) => { void openWhisper(w); }}
                  onRead={readWhisper}
                  onArchive={archiveWhisper}
                />
              ),
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default MentionsList;
