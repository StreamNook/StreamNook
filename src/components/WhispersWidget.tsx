// The desktop Whispers window: every conversation of the signed-in Twitch
// account and the open one's history. Rust owns all of it (the list, the
// messages, read state, sending and whether a send arrived); this draws the
// loaded page, keeps the draft, and sends what the viewer does.
import { useState, useEffect, useLayoutEffect, useRef, useCallback, useMemo, type CSSProperties } from 'react';
import { motion } from 'framer-motion';
import { X, MessageCircle, Send, Search, Plus, ArrowLeft, Loader2, Users, Smile, User, Archive, RefreshCw, MoreHorizontal, Play } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { WHISPER_ARCHIVE_CHANGED_EVENT, WHISPER_UPDATED_EVENT, useInboxCounts } from '../stores/mentionInboxStore';
import { getAppleEmojiUrl } from '../services/emojiService';
import { Tooltip } from './ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, COMPOSER_STRIP, COMPOSER_STRIP_STYLE, GLAZE_BANNER, GLAZE_PILL } from './ui/glazeButtons';
import SpellcheckUnderlay from './chat/SpellcheckUnderlay';
import { PinnedChatter } from './chat/PinnedChatter';
import { WhisperMessages } from './whispers/WhisperMessages';
import { WhisperParts } from './whispers/WhisperParts';
import {
    applyMessageUpdate,
    applyRowUpdate,
    archiveOwnerOf,
    archiveSyncing,
    counterText,
    draftRow,
    filterRows,
    isOverLimit,
    liveLine,
    mergeDraft,
    mergeMessages,
    messageTime,
    ownerChanged,
    sortRows,
    syncLine,
    syncStatusWords,
} from './whispers/whisperView';
import { usePerformanceQuiet } from '../utils/lowPower';
import { useSpellcheck } from '../hooks/useSpellcheck';
import { warmSpellcheck } from '../utils/spellcheck';
import { claimWhisper } from '../utils/whisperClaim';
import type {
    StoredWhisperMessage,
    UserInfo,
    WhisperListPayload,
    WhisperPagePayload,
    WhisperRowMeta,
    WhisperSyncOutcome,
    WhisperUpdatePayload,
} from '../types';
import { Logger } from '../utils/logger';
import { SevenTvAvatar } from './SevenTvAvatar';
import { isImeComposing } from '../utils/imeComposing';
import { useOutsidePress } from '../hooks/useOutsidePress';

interface WhispersWidgetProps {
    isOpen: boolean;
    onClose: () => void;
}

/** Messages a conversation opens with, and each older page. */
const PAGE = 50;
/** `whisper_page` caps a page here. */
const PAGE_MAX = 200;
/** How close to the top an older page starts loading. */
const LOAD_OLDER_PX = 80;
/** How close to the bottom still counts as reading the newest message. */
const AT_BOTTOM_PX = 120;
/** How long a refresh's words stay. */
const STATUS_MS = 3000;

// Emoji categories for the picker
const emojiCategories: Record<string, string[]> = {
    'Smileys': ['😀', '😃', '😄', '😁', '😅', '😂', '🤣', '😊', '😇', '🙂', '🙃', '😉', '😌', '😍', '🥰', '😘', '😗', '😙', '😚', '😋', '😛', '😜', '🤪', '😝', '🤑', '🤗', '🤭', '🤫', '🤔', '🤐', '🤨', '😐', '😑', '😶', '😏', '😒', '🙄', '😬', '🤥', '😔', '😪', '🤤', '😴', '😷', '🤒', '🤕', '🤢', '🤮', '🤧', '🥵', '🥶', '🥴', '😵', '🤯', '🤠', '🥳', '🥸', '😎', '🤓', '🧐'],
    'Gestures': ['👍', '👎', '👊', '✊', '🤛', '🤜', '👏', '🙌', '👐', '🤲', '🤝', '🙏', '✌️', '🤞', '🤟', '🤘', '🤙', '👈', '👉', '👆', '👇', '☝️', '👋', '🤚', '🖐️', '✋', '🖖', '💪', '🦾', '🦿', '🦵', '🦶', '👂', '🦻', '👃', '🧠', '🫀', '🫁', '🦷', '🦴', '👀', '👁️', '👅', '👄'],
    'Hearts': ['❤️', '🧡', '💛', '💚', '💙', '💜', '🖤', '🤍', '🤎', '💔', '❣️', '💕', '💞', '💓', '💗', '💖', '💘', '💝', '💟', '♥️', '💌', '💋'],
    'Animals': ['🐶', '🐱', '🐭', '🐹', '🐰', '🦊', '🐻', '🐼', '🐨', '🐯', '🦁', '🐮', '🐷', '🐸', '🐵', '🙈', '🙉', '🙊', '🐔', '🐧', '🐦', '🐤', '🦆', '🦅', '🦉', '🦇', '🐺', '🐗', '🐴', '🦄', '🐝', '🐛', '🦋', '🐌', '🐞', '🐜', '🦟', '🦗', '🕷️', '🦂', '🐢', '🐍', '🦎', '🦖', '🦕', '🐙', '🦑', '🦐', '🦞', '🦀', '🐡', '🐠', '🐟', '🐬', '🐳', '🐋', '🦈', '🐊'],
    'Food': ['🍎', '🍐', '🍊', '🍋', '🍌', '🍉', '🍇', '🍓', '🫐', '🍈', '🍒', '🍑', '🥭', '🍍', '🥥', '🥝', '🍅', '🍆', '🥑', '🥦', '🥬', '🥒', '🌶️', '🫑', '🌽', '🥕', '🫒', '🧄', '🧅', '🥔', '🍠', '🥐', '🥯', '🍞', '🥖', '🥨', '🧀', '🥚', '🍳', '🧈', '🥞', '🧇', '🥓', '🥩', '🍗', '🍖', '🦴', '🌭', '🍔', '🍟', '🍕', '🫓', '🥪', '🥙', '🧆', '🌮', '🌯', '🫔', '🥗'],
    'Activities': ['⚽', '🏀', '🏈', '⚾', '🥎', '🎾', '🏐', '🏉', '🥏', '🎱', '🪀', '🏓', '🏸', '🏒', '🏑', '🥍', '🏏', '🪃', '🥅', '⛳', '🪁', '🏹', '🎣', '🤿', '🥊', '🥋', '🎽', '🛹', '🛼', '🛷', '⛸️', '🥌', '🎿', '⛷️', '🏂', '🪂', '🏋️', '🤼', '🤸', '🤺', '⛹️', '🤾', '🏌️', '🏇', '🧘', '🏄', '🏊', '🤽', '🚣', '🧗', '🚵', '🚴', '🏆', '🥇', '🥈', '🥉'],
    'Objects': ['⌚', '📱', '💻', '⌨️', '🖥️', '💡', '🔦', '💎', '🔮', '🎮', '🎯', '🎨', '🎭', '🎪', '🎤', '🎧', '🎼', '🎹', '🥁', '🎷', '🎺', '🎸', '🪕', '🎻', '🎬', '🏹'],
    'Symbols': ['❤️', '💯', '✨', '⭐', '🔥', '💥', '✅', '❌', '➕', '➖', '✖️', '➡️', '⬅️', '⬆️', '⬇️', '🔴', '🟠', '🟡', '🟢', '🔵', '🟣', '⚫', '⚪', '🟤']
};

interface OpenPage {
    key: string;
    meta: WhisperRowMeta | null;
    messages: StoredWhisperMessage[];
    cursor: string | null;
    hasMoreLocal: boolean;
    olderOnTwitch: boolean;
}

const pageFrom = (key: string, p: WhisperPagePayload): OpenPage => ({
    key,
    meta: p.meta,
    messages: p.messages,
    cursor: p.cursor,
    hasMoreLocal: p.has_more_local,
    olderOnTwitch: p.older_on_twitch,
});

/** A conversation's face: the picture (a live ring while they stream), or
 *  the whisper mark while there is none. 46 px in the list, 48 in the header. */
function Face({ row, size }: { row: WhisperRowMeta; size: 46 | 48 }) {
    const ring = row.live ? 'ring-2 ring-live/80' : '';
    return row.profile_image_url ? (
        <SevenTvAvatar
            cosmeticsId={row.user_id}
            px={size}
            motion="hover"
            src={row.profile_image_url}
            alt={row.user_name}
            className={`rounded-full object-cover ${ring}`}
            style={{ width: size, height: size }}
        />
    ) : (
        <div
            className={`rounded-full bg-accent/10 flex items-center justify-center ${ring}`}
            style={{ width: size, height: size }}
        >
            <MessageCircle size={18} className="text-accent" />
        </div>
    );
}

/** The accent count on a row: no corner marks, "99+" past 99. */
function UnreadCapsule({ count }: { count: number }) {
    return (
        <span
            className="min-w-[20px] h-5 px-1.5 rounded-full text-[11.5px] font-bold leading-5 text-center tabular-nums shrink-0"
            style={{ background: 'var(--color-accent)', color: 'color-mix(in srgb, var(--color-accent) 14%, black)' }}
        >
            {count > 99 ? '99+' : count}
        </span>
    );
}

/** The open conversation's row: soft glass in the accent's tint. */
const SELECTED_ROW_STYLE: CSSProperties = {
    background: 'color-mix(in srgb, var(--color-accent) 14%, transparent)',
    boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 22%, transparent)',
};

/** The composer bar: the glaze the strips above composers wear, rounder. */
const COMPOSER_BAR_STYLE = { borderRadius: 16, '--glaze-frost': '24%' } as CSSProperties;

const WhispersWidget = ({ isOpen, onClose }: WhispersWidgetProps) => {
    const whisperTargetUser = useAppStore((s) => s.whisperTargetUser);
    const clearWhisperTargetUser = useAppStore((s) => s.clearWhisperTargetUser);
    const setProfileModalUser = useAppStore((s) => s.setProfileModalUser);
    const spellcheckEnabled = useAppStore((s) => s.settings.chat_input?.spellcheck_enabled ?? true);
    const inboxCounts = useInboxCounts();
    // Motion Off and Performance mode open the window and draw new bubbles at
    // once; otherwise each eases in over 140 ms.
    const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
    const performanceQuiet = usePerformanceQuiet();
    const instant = motionOff || performanceQuiet;

    const [list, setList] = useState<WhisperListPayload | null>(null);
    const [draft, setDraft] = useState<WhisperRowMeta | null>(null);
    const [activeKey, setActiveKey] = useState<string | null>(null);
    const [page, setPage] = useState<OpenPage | null>(null);
    const [loadingOlder, setLoadingOlder] = useState(false);
    const [olderNote, setOlderNote] = useState<string | null>(null);
    const [message, setMessage] = useState('');
    const [isSending, setIsSending] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [searchQuery, setSearchQuery] = useState('');
    const [isSearching, setIsSearching] = useState(false);
    const [searchResults, setSearchResults] = useState<UserInfo[]>([]);
    const [showNewConversation, setShowNewConversation] = useState(false);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [status, setStatus] = useState<string | null>(null);
    const [listFilter, setListFilter] = useState('');
    const [showMore, setShowMore] = useState(false);
    const [syncedAt, setSyncedAt] = useState<number | null>(null);
    const [showEmojiPicker, setShowEmojiPicker] = useState(false);
    const [emojiSearchQuery, setEmojiSearchQuery] = useState('');
    const [selectedEmojiCategory, setSelectedEmojiCategory] = useState<string>('Smileys');
    const [now, setNow] = useState(() => Date.now());

    const scrollRef = useRef<HTMLDivElement>(null);
    const inputRef = useRef<HTMLTextAreaElement>(null);
    const moreRef = useRef<HTMLDivElement>(null);
    const emojiPickerRef = useRef<HTMLDivElement>(null);
    const emojiButtonRef = useRef<HTMLButtonElement>(null);
    const sendingRef = useRef(false);
    const loadingOlderRef = useRef(false);
    const statusTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
    // Kept in step with state for the event handlers and async work below.
    const listRef = useRef<WhisperListPayload | null>(null);
    const pageRef = useRef<OpenPage | null>(null);
    const activeRef = useRef<string | null>(null);
    const ownerRef = useRef<string | null>(null);
    // Scroll handling across renders: keep the reader's place when an older
    // page joins above, and follow the newest message while they are at it.
    const keepScroll = useRef<{ height: number; top: number } | null>(null);
    const stickToBottom = useRef(true);

    useEffect(() => { listRef.current = list; }, [list]);
    useEffect(() => { pageRef.current = page; }, [page]);
    useEffect(() => { activeRef.current = activeKey; }, [activeKey]);

    // Spell check for the composer. No channel here, so there is no emote set
    // to consult: only the custom dictionary and the chatter list apply.
    const spellUnderlayRef = useRef<HTMLDivElement>(null);
    const spellRanges = useSpellcheck(message, { enabled: spellcheckEnabled, emoteKey: null });

    const showStatus = useCallback((words: string | null) => {
        if (statusTimer.current) clearTimeout(statusTimer.current);
        setStatus(words);
        if (words) statusTimer.current = setTimeout(() => setStatus(null), STATUS_MS);
    }, []);
    useEffect(() => () => {
        if (statusTimer.current) clearTimeout(statusTimer.current);
    }, []);

    // Times say "now" relative to a clock that moves once a minute while open.
    useEffect(() => {
        if (!isOpen) return;
        const tick = setInterval(() => setNow(Date.now()), 60_000);
        return () => clearInterval(tick);
    }, [isOpen]);

    const reloadList = useCallback(async () => {
        try {
            setList(await invoke<WhisperListPayload>('whisper_list'));
        } catch (e) {
            Logger.warn('[Whispers] whisper_list failed:', e);
        }
    }, []);

    // The newest page of a conversation, if it is still the one open.
    const loadLatest = useCallback(async (key: string, limit: number = PAGE) => {
        try {
            const p = await invoke<WhisperPagePayload>('whisper_page', { key, cursor: null, limit: Math.min(limit, PAGE_MAX) });
            if (activeRef.current === key) setPage(pageFrom(key, p));
        } catch (e) {
            Logger.warn('[Whispers] whisper_page failed:', e);
        }
    }, []);

    // While open: the list, and every change Rust reports. A change lands as the
    // one conversation it touched; a sync or many changes reload the list.
    useEffect(() => {
        if (!isOpen) return;
        void reloadList();
        // The window opening is an inbox opening: Rust syncs with Twitch if a
        // minute has passed since the last time.
        invoke('inbox_opened').catch((e) => Logger.debug('[Whispers] inbox_opened failed:', e));
        let alive = true;
        const offUpdated = listen<WhisperUpdatePayload>(WHISPER_UPDATED_EVENT, (event) => {
            if (!alive) return;
            const update = event.payload;
            // Another account's change: the account switched under the window,
            // so the list is read again (and the old one's conversation closes).
            if (ownerChanged(listRef.current?.owner_id, update.owner_id)) {
                void reloadList();
                return;
            }
            setList((prev) => applyRowUpdate(prev, update));
            setDraft((d) => (d && (d.key === update.key || d.user_id === update.meta.user_id) ? null : d));
            setActiveKey((a) => (a !== null && a === update.replaced_key ? update.key : a));
            setPage((prev) => {
                if (!prev || (prev.key !== update.key && prev.key !== update.replaced_key)) return prev;
                return { ...prev, key: update.key, meta: update.meta, messages: applyMessageUpdate(prev.messages, update) };
            });
            if (update.error && activeRef.current === update.key) setOlderNote(update.error);
        });
        const offArchive = listen<unknown>(WHISPER_ARCHIVE_CHANGED_EVENT, (event) => {
            if (!alive) return;
            if (ownerChanged(listRef.current?.owner_id, archiveOwnerOf(event.payload))) {
                void reloadList();
                return;
            }
            const syncing = archiveSyncing(event.payload);
            setList((prev) => (prev ? { ...prev, syncing } : prev));
            if (!syncing) setSyncedAt(Date.now());
            void reloadList();
            const key = activeRef.current;
            if (key) void loadLatest(key, Math.max(PAGE, pageRef.current?.messages.length ?? 0));
        });
        return () => {
            alive = false;
            void offUpdated.then((fn) => fn());
            void offArchive.then((fn) => fn());
        };
    }, [isOpen, reloadList, loadLatest]);

    // The app's Twitch account changed while the window is open: read the list
    // for the account now signed in.
    const currentUserId = useAppStore((s) => s.currentUser?.user_id ?? null);
    useEffect(() => {
        if (!isOpen) return;
        if (ownerChanged(listRef.current?.owner_id, currentUserId)) void reloadList();
    }, [isOpen, currentUserId, reloadList]);

    // A different account (or signing out) drops what the old one had open.
    useEffect(() => {
        const owner = list?.owner_id ?? null;
        if (ownerRef.current !== null && owner !== ownerRef.current) {
            setActiveKey(null);
            setDraft(null);
            setPage(null);
        }
        ownerRef.current = owner;
    }, [list?.owner_id]);

    // Tell Rust which conversation this window shows, so a whisper landing in
    // it while the window is focused counts as read. Every close path ends up
    // here with isOpen false, and unmounting clears it too.
    useEffect(() => {
        claimWhisper(isOpen ? activeKey : null);
    }, [isOpen, activeKey]);
    useEffect(() => () => claimWhisper(null), []);

    // Opening a conversation: its newest page, read on open.
    useEffect(() => {
        if (!isOpen || !activeKey) {
            setPage(null);
            return;
        }
        setPage((prev) => (prev?.key === activeKey ? prev : null));
        setOlderNote(null);
        stickToBottom.current = true;
        void loadLatest(activeKey);
        const row = listRef.current?.conversations.find((r) => r.key === activeKey);
        if (row && row.unread_count > 0) {
            invoke('whisper_mark_read', { key: activeKey }).catch((err) => Logger.warn('[Whispers] Failed to mark read:', err));
        }
        inputRef.current?.focus({ preventScroll: true });
    }, [isOpen, activeKey, loadLatest]);

    // A draft lives only while it is the conversation on screen.
    useEffect(() => {
        setDraft((d) => (d && d.key !== activeKey ? null : d));
    }, [activeKey]);

    // The profile card's Whisper button, a toast's Reply, the island's row,
    // MultiChat's row: open that person's conversation, or a new one.
    useEffect(() => {
        if (!isOpen || !whisperTargetUser) return;
        const target = whisperTargetUser;
        const known = listRef.current?.conversations.find((r) => r.user_id === target.id || r.key === target.id);
        if (known) {
            setDraft(null);
            setActiveKey(known.key);
        } else {
            setDraft(draftRow(target));
            setActiveKey(target.id);
        }
        setShowNewConversation(false);
        clearWhisperTargetUser();
    }, [isOpen, whisperTargetUser, clearWhisperTargetUser]);

    // Keep the reader's place: at the newest message when they were there, or
    // where they were when an older page joined above.
    useLayoutEffect(() => {
        const el = scrollRef.current;
        if (!el || !page) return;
        if (keepScroll.current) {
            el.scrollTop = el.scrollHeight - keepScroll.current.height + keepScroll.current.top;
            keepScroll.current = null;
        } else if (stickToBottom.current) {
            el.scrollTop = el.scrollHeight;
        }
    }, [page]);

    // The page before the oldest loaded one. Callers hold loadingOlderRef.
    const fetchOlder = useCallback(async (current: OpenPage) => {
        if (!current.cursor) return;
        const p = await invoke<WhisperPagePayload>('whisper_page', { key: current.key, cursor: current.cursor, limit: PAGE });
        const el = scrollRef.current;
        if (el) keepScroll.current = { height: el.scrollHeight, top: el.scrollTop };
        setPage((prev) =>
            prev && prev.key === current.key
                ? {
                      ...prev,
                      messages: mergeMessages(p.messages, prev.messages),
                      cursor: p.cursor ?? prev.cursor,
                      hasMoreLocal: p.has_more_local,
                      olderOnTwitch: p.older_on_twitch,
                  }
                : prev,
        );
    }, []);

    const loadOlderLocal = useCallback(async () => {
        const current = pageRef.current;
        if (!current || !current.hasMoreLocal || loadingOlderRef.current) return;
        loadingOlderRef.current = true;
        setLoadingOlder(true);
        try {
            await fetchOlder(current);
        } catch (e) {
            Logger.warn('[Whispers] older page failed:', e);
        } finally {
            loadingOlderRef.current = false;
            setLoadingOlder(false);
        }
    }, [fetchOlder]);

    // History older than this device has, from Twitch (one page a tap).
    const loadFromTwitch = useCallback(async () => {
        const current = pageRef.current;
        if (!current || loadingOlderRef.current) return;
        loadingOlderRef.current = true;
        setLoadingOlder(true);
        setOlderNote(null);
        try {
            await invoke<number>('whisper_load_older', { key: current.key });
            await fetchOlder(current);
        } catch (e) {
            setOlderNote(typeof e === 'string' && e.trim() ? e : 'Could not load older messages from Twitch.');
        } finally {
            loadingOlderRef.current = false;
            setLoadingOlder(false);
        }
    }, [fetchOlder]);

    const onMessagesScroll = (e: React.UIEvent<HTMLDivElement>) => {
        const el = e.currentTarget;
        stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < AT_BOTTOM_PX;
        if (el.scrollTop < LOAD_OLDER_PX) void loadOlderLocal();
    };

    const handleRefresh = async () => {
        if (isRefreshing) return;
        setIsRefreshing(true);
        try {
            showStatus(syncStatusWords(await invoke<WhisperSyncOutcome>('whisper_sync_now')));
        } catch (e) {
            showStatus(typeof e === 'string' && e.trim() ? `Could not refresh: ${e}` : 'Could not refresh.');
        } finally {
            setIsRefreshing(false);
        }
    };

    const signIn = () => {
        void useAppStore.getState().loginToTwitch();
    };

    // Close menus when clicking outside
    const markInsideMore = useOutsidePress(showMore, [moreRef], () => setShowMore(false));
    const markInsideEmojiPicker = useOutsidePress(showEmojiPicker, [emojiPickerRef, emojiButtonRef], () => setShowEmojiPicker(false));

    const handleClose = useCallback(() => {
        setActiveKey(null);
        setDraft(null);
        onClose();
    }, [onClose]);

    // Search for users
    const handleSearch = async () => {
        if (!searchQuery.trim()) { setSearchResults([]); return; }
        setIsSearching(true);
        try {
            try {
                const found = await invoke<[string, string, string, string | null] | null>('search_whisper_user', { username: searchQuery });
                if (found) {
                    const [userId, userLogin, displayName, profileImageUrl] = found;
                    setSearchResults([{ id: userId, login: userLogin, display_name: displayName, profile_image_url: profileImageUrl || undefined }]);
                    return;
                }
            } catch { /* fall back to the channel search */ }
            const results = await invoke<Record<string, unknown>[]>('search_channels', { query: searchQuery });
            setSearchResults(results.slice(0, 10).map((r: Record<string, unknown>) => ({ id: (r.user_id || r.id) as string, login: (r.user_login || r.login) as string, display_name: (r.user_name || r.display_name) as string, profile_image_url: r.profile_image_url as string | undefined })).filter(r => r.id && r.login));
        } catch { setSearchResults([]); }
        finally { setIsSearching(false); }
    };

    const startConversation = (user: UserInfo) => {
        const known = listRef.current?.conversations.find((r) => r.user_id === user.id || r.key === user.id);
        if (known) {
            setActiveKey(known.key);
        } else {
            setDraft(draftRow(user));
            setActiveKey(user.id);
        }
        setShowNewConversation(false);
        setSearchQuery('');
        setSearchResults([]);
    };

    const allRows = useMemo(() => sortRows(mergeDraft(list?.conversations ?? [], draft), 'recent'), [list, draft]);
    const rows = useMemo(() => filterRows(allRows, listFilter), [allRows, listFilter]);
    const activeRow = activeKey ? allRows.find((r) => r.key === activeKey) ?? page?.meta ?? null : null;
    const totalUnread = inboxCounts?.whispers ?? list?.unread_total ?? 0;

    const handleSend = async () => {
        const text = message.trim();
        if (!text || isOverLimit(text) || sendingRef.current || !activeRow) return;
        sendingRef.current = true;
        setIsSending(true);
        setError(null);
        setShowEmojiPicker(false);
        setMessage('');
        const target = {
            key: activeRow.key,
            user_id: activeRow.user_id,
            user_login: activeRow.user_login,
            user_name: activeRow.user_name,
            profile_image_url: activeRow.profile_image_url,
        };
        stickToBottom.current = true;
        try {
            // Bound to the account whose list is on screen: Rust refuses the send
            // if the signed-in account is another one by now.
            const expectOwner = listRef.current?.owner_id || null;
            const update = await invoke<WhisperUpdatePayload>('whisper_send', { target, text, expectOwner, expectOwnerName: null });
            setList((prev) => applyRowUpdate(prev, update));
            setPage((prev) =>
                prev && (prev.key === update.key || prev.key === target.key)
                    ? { ...prev, key: update.key, meta: update.meta, messages: applyMessageUpdate(prev.messages, update) }
                    : prev,
            );
            if (update.key !== target.key) setActiveKey(update.key);
        } catch (err) {
            // Rust files a whisper before it goes out, so a refusal from Twitch
            // shows under the message itself, with Retry. One refused before it
            // was filed (signed out, too long) gets its words back here.
            const filed = pageRef.current?.messages.some((m) => m.is_sent && m.message === text) ?? false;
            if (!filed) {
                setError(typeof err === 'string' && err.trim() ? err : 'Could not send this whisper.');
                setMessage((m) => (m ? m : text));
            }
        } finally {
            setIsSending(false);
            sendingRef.current = false;
            inputRef.current?.focus({ preventScroll: true });
        }
    };

    const retry = (m: StoredWhisperMessage) => {
        if (!activeRow) return;
        invoke('whisper_retry', { key: activeRow.key, messageId: m.id }).catch((err) => {
            Logger.warn('[Whispers] retry failed:', err);
            setError(typeof err === 'string' && err.trim() ? err : 'Could not send this whisper.');
        });
    };

    const discard = (m: StoredWhisperMessage) => {
        if (!activeRow) return;
        invoke('whisper_discard', { key: activeRow.key, messageId: m.id }).catch((err) =>
            Logger.warn('[Whispers] discard failed:', err),
        );
    };

    // Archive is Twitch's own: the conversation leaves the list here and on
    // twitch.tv until someone whispers again. A draft just goes.
    const archiveConversation = (row: WhisperRowMeta) => {
        setList((prev) => (prev ? { ...prev, conversations: prev.conversations.filter((r) => r.key !== row.key) } : prev));
        if (activeKey === row.key) setActiveKey(null);
        if (draft?.key === row.key) {
            setDraft(null);
            return;
        }
        invoke('whisper_archive', { key: row.key }).catch((err) => Logger.warn('[Whispers] archive failed:', err));
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter' && !e.shiftKey && !isImeComposing(e)) {
            e.preventDefault();
            void handleSend();
        }
    };

    const insertEmoji = (emoji: string) => {
        setMessage(prev => prev + emoji);
        inputRef.current?.focus({ preventScroll: true });
    };

    const getFilteredEmojis = () => {
        const categoryEmojis = emojiCategories[selectedEmojiCategory] || [];
        if (!emojiSearchQuery) return categoryEmojis;
        const allEmojis = Object.values(emojiCategories).flat();
        return allEmojis.filter(emoji => emoji.includes(emojiSearchQuery));
    };

    const handleProfileClick = () => {
        if (!activeRow) return;
        setProfileModalUser({
            id: activeRow.user_id,
            user_id: activeRow.user_id,
            user_name: activeRow.user_name,
            user_login: activeRow.user_login,
            title: `${activeRow.user_name}'s Profile`,
            viewer_count: 0,
            game_name: '',
            thumbnail_url: activeRow.profile_image_url || '',
            started_at: '',
            profile_image_url: activeRow.profile_image_url || '',
        });
    };

    // Watch: their live channel in the player, through the app's own open path.
    const watchLive = () => {
        if (!activeRow?.live) return;
        void useAppStore.getState().startStream(activeRow.user_login);
        handleClose();
    };

    if (!isOpen) return null;

    const counter = counterText(message);
    const over = isOverLimit(message);
    const syncing = isRefreshing || (list?.syncing ?? false);
    const signedOut = list !== null && !list.signed_in;
    const live = liveLine(activeRow?.live);
    const ease = instant ? { duration: 0 } : { duration: 0.16, ease: 'easeOut' as const };

    return (
        <motion.div
            initial={instant ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={ease}
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
            onClick={handleClose}
        >
            <motion.div
                initial={instant ? false : { scale: 0.98, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                exit={{ scale: 0.98, opacity: 0 }}
                transition={ease}
                className="glass-modal relative flex flex-col overflow-hidden"
                // Roomy: about 1120 x 700 on a common window, never under
                // 880 x 560 (or the screen) and never over 1240 x 780.
                style={{
                    width: 'clamp(min(880px, 94vw), 88vw, 1240px)',
                    height: 'clamp(min(560px, 90vh), 82vh, 780px)',
                    borderRadius: 22,
                }}
                onClick={(e) => e.stopPropagation()}
            >
                {/* Header: the title, the sync line, search, New whisper, Close.
                    Controls sit on the title's cap-height midpoint (items-center
                    on one 60 px row) and read a step quieter than the title. */}
                <div className="flex h-[60px] shrink-0 items-center gap-3.5 border-b border-borderSubtle pl-[22px] pr-[22px]">
                    <span className="text-[17px] font-bold leading-none text-textPrimary">Whispers</span>
                    {totalUnread > 0 && <UnreadCapsule count={totalUnread} />}
                    {!signedOut && (
                        <span className="truncate text-[12.5px] leading-none text-textMuted" aria-live="polite">
                            {status ?? syncLine(syncing, syncedAt, now)}
                        </span>
                    )}
                    <span className="flex-1" />
                    <label className="glass-input flex h-9 w-[280px] shrink-0 items-center gap-2 rounded-xl px-3 text-textMuted">
                        <Search size={16} className="shrink-0" aria-hidden />
                        <input
                            type="text"
                            value={listFilter}
                            onChange={(e) => setListFilter(e.target.value)}
                            placeholder="Search people"
                            aria-label="Search whispers"
                            className="min-w-0 flex-1 bg-transparent text-[13.5px] text-textPrimary outline-none placeholder:text-textMuted"
                        />
                    </label>
                    <Tooltip content="Refresh from Twitch" side="bottom">
                        <button
                            type="button"
                            aria-label="Refresh from Twitch"
                            onClick={handleRefresh}
                            disabled={isRefreshing || signedOut}
                            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary disabled:opacity-50"
                        >
                            <RefreshCw size={14} className={syncing && !instant ? 'animate-spin' : ''} />
                        </button>
                    </Tooltip>
                    <Tooltip content="New whisper" side="bottom">
                        <button
                            type="button"
                            aria-label="New whisper"
                            onClick={() => setShowNewConversation(true)}
                            disabled={signedOut}
                            className={`${GLAZE_PILL} flex h-9 w-9 shrink-0 items-center justify-center !p-0 disabled:opacity-50`}
                        >
                            <Plus size={17} />
                        </button>
                    </Tooltip>
                    <button
                        type="button"
                        aria-label="Close"
                        onClick={handleClose}
                        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
                    >
                        <X size={18} />
                    </button>
                </div>

                {list?.session_problem && (
                    <div className="shrink-0 px-[22px] pt-3">
                        <div className={COMPOSER_STRIP} style={COMPOSER_STRIP_STYLE}>
                            <span className="min-w-0 flex-1 text-[12.5px] leading-[18px] text-textPrimary">{list.session_problem}</span>
                            <button type="button" onClick={signIn} className={`${GLAZE_PILL} h-7 shrink-0 px-3 text-[12.5px]`}>
                                Sign in
                            </button>
                        </div>
                    </div>
                )}

                <div className="flex min-h-0 flex-1">
                {/* The conversations: 330 px, rows of 64 px */}
                <div className="flex w-[330px] shrink-0 flex-col border-r border-borderSubtle min-h-0">
                    <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-2.5 pt-2.5 pb-7 scrollbar-thin" style={{ maskImage: 'linear-gradient(to bottom, black calc(100% - 28px), transparent)', WebkitMaskImage: 'linear-gradient(to bottom, black calc(100% - 28px), transparent)' }}>
                        {signedOut ? (
                            <div className="flex h-full flex-col items-center justify-center p-6 text-center text-textMuted">
                                <MessageCircle size={40} className="mb-3 text-accent/50 opacity-50" />
                                <span className="mb-3 text-sm">Sign in to Twitch to see your whispers</span>
                                <button type="button" onClick={signIn} className={`${GLAZE_PILL} h-8 px-4 text-[13px]`}>
                                    Sign in
                                </button>
                            </div>
                        ) : rows.length === 0 ? (
                            <div className="flex h-full flex-col items-center justify-center p-6 text-textMuted">
                                <Users size={40} className="mb-3 text-accent/50 opacity-50" />
                                <span className="mb-2 text-center text-sm">
                                    {listFilter.trim() ? 'Nobody by that name' : syncing ? 'Loading your whispers...' : 'No conversations yet'}
                                </span>
                                {!syncing && !listFilter.trim() && (
                                    <button type="button" onClick={() => setShowNewConversation(true)} className="text-sm font-medium text-accent transition-colors">
                                        Start a new conversation
                                    </button>
                                )}
                            </div>
                        ) : (
                            rows.map((row) => {
                                const selected = activeKey === row.key;
                                const unread = row.unread_count > 0;
                                return (
                                    <motion.div
                                        key={row.key}
                                        initial={instant ? false : { opacity: 0, y: 4 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        transition={instant ? { duration: 0 } : { duration: 0.14, ease: 'easeOut' }}
                                        role="button"
                                        tabIndex={0}
                                        data-whisper-row={row.key}
                                        aria-current={selected ? 'true' : undefined}
                                        className={`group relative flex h-16 shrink-0 cursor-pointer items-center gap-3 rounded-[14px] px-3 transition-colors ${
                                            selected ? '' : 'hover:bg-white/[0.04]'
                                        }`}
                                        style={selected ? SELECTED_ROW_STYLE : undefined}
                                        onClick={() => setActiveKey(row.key)}
                                        onKeyDown={(e) => {
                                            if (e.target !== e.currentTarget) return;
                                            if (e.key === 'Enter' || e.key === ' ') {
                                                e.preventDefault();
                                                setActiveKey(row.key);
                                            }
                                        }}
                                    >
                                        <Face row={row} size={46} />
                                        <div className="flex min-w-0 flex-1 flex-col gap-[3px]">
                                            <div className="flex min-w-0 items-center gap-1.5">
                                                <span className="flex min-w-0 items-center gap-1 text-[14.5px] leading-5 text-textPrimary">
                                                    <PinnedChatter
                                                        provider="twitch"
                                                        userId={row.user_id}
                                                        name={row.user_name}
                                                        login={row.user_login}
                                                        color={row.color ?? undefined}
                                                        badges={row.badges}
                                                        variant="line"
                                                        fold
                                                        nameClassName="min-w-0 shrink-0 max-w-full truncate font-bold"
                                                    />
                                                </span>
                                                <span className="flex-1" />
                                                {/* The time, swapped for Archive on hover: one fixed slot,
                                                    so the row never reflows between the two. */}
                                                <span className="relative flex h-5 w-[58px] shrink-0 items-center justify-end">
                                                    <span className="relative top-px text-[11.5px] tabular-nums text-textMuted transition-opacity group-hover:opacity-0 group-focus-within:opacity-0">
                                                        {messageTime(row.last_message_timestamp, now)}
                                                    </span>
                                                    <Tooltip content="Archive" side="top">
                                                        <button
                                                            type="button"
                                                            aria-label="Archive"
                                                            onClick={(e) => {
                                                                e.stopPropagation();
                                                                archiveConversation(row);
                                                            }}
                                                            className="absolute -right-1 top-1/2 -translate-y-1/2 rounded-lg p-1 text-textMuted opacity-0 transition-opacity hover:bg-white/[0.06] hover:text-textPrimary group-hover:opacity-100 group-focus-within:opacity-100"
                                                        >
                                                            <Archive size={14} />
                                                        </button>
                                                    </Tooltip>
                                                </span>
                                            </div>
                                            <div className="flex min-w-0 items-center gap-2">
                                                <span className={`min-w-0 flex-1 truncate text-[13px] leading-[18px] ${unread ? 'font-semibold text-textPrimary' : 'text-textMuted'}`}>
                                                    {row.last_is_sent && <span className="font-normal text-textMuted">You: </span>}
                                                    {row.last_text || row.last_parts.length > 0 ? (
                                                        <WhisperParts parts={row.last_parts} text={row.last_text} emoteClass="mx-px inline-block h-[1.2em] w-auto align-middle" />
                                                    ) : (
                                                        'No messages yet'
                                                    )}
                                                </span>
                                                {unread && <UnreadCapsule count={row.unread_count} />}
                                            </div>
                                        </div>
                                    </motion.div>
                                );
                            })
                        )}
                    </div>
                </div>

                {/* The open conversation, a new one, or the empty pane */}
                <div className="flex min-w-0 flex-1 flex-col">
                    {showNewConversation ? (
                        <div className="flex min-h-0 flex-1 flex-col">
                            <div className="flex h-[78px] shrink-0 items-center gap-3 border-b border-borderSubtle px-[22px]">
                                <button
                                    type="button"
                                    aria-label="Back"
                                    onClick={() => { setShowNewConversation(false); setSearchQuery(''); setSearchResults([]); }}
                                    className="flex h-9 w-9 items-center justify-center rounded-xl text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
                                >
                                    <ArrowLeft size={18} />
                                </button>
                                <span className="text-[17px] font-bold text-textPrimary">New whisper</span>
                            </div>
                            <div className="shrink-0 px-[22px] pt-5">
                                <div className="flex items-center gap-3">
                                    <label className="glass-input flex h-10 flex-1 items-center gap-2 rounded-xl px-3 text-textMuted">
                                        <Search size={16} className="shrink-0" aria-hidden />
                                        <input
                                            type="text"
                                            value={searchQuery}
                                            onChange={(e) => setSearchQuery(e.target.value)}
                                            onKeyDown={(e) => e.key === 'Enter' && !isImeComposing(e) && handleSearch()}
                                            placeholder="Who do you want to whisper?"
                                            aria-label="Search for someone"
                                            className="min-w-0 flex-1 bg-transparent text-[14px] text-textPrimary outline-none placeholder:text-textMuted"
                                        />
                                    </label>
                                    <button
                                        type="button"
                                        onClick={handleSearch}
                                        disabled={isSearching}
                                        className={`${ACCENT_BUTTON} h-10 px-5`}
                                        style={ACCENT_FILL}
                                    >
                                        {isSearching ? <Loader2 size={16} className={instant ? '' : 'animate-spin'} /> : 'Search'}
                                    </button>
                                </div>
                            </div>
                            <div className="min-h-0 flex-1 overflow-y-auto px-[22px] py-4">
                                {searchResults.length > 0 ? (
                                    <div className="flex flex-col gap-1">
                                        {searchResults.map((user) => (
                                            <button
                                                type="button"
                                                key={user.id}
                                                onClick={() => startConversation(user)}
                                                className="flex h-16 items-center gap-3 rounded-[14px] px-3 text-left transition-colors hover:bg-white/[0.04]"
                                            >
                                                <Face row={draftRow(user)} size={46} />
                                                <span className="flex min-w-0 flex-col">
                                                    <span className="truncate text-[14.5px] font-bold text-textPrimary">{user.display_name}</span>
                                                    <span className="truncate text-[13px] text-textMuted">@{user.login}</span>
                                                </span>
                                            </button>
                                        ))}
                                    </div>
                                ) : searchQuery && !isSearching ? (
                                    <div className="py-8 text-center text-textMuted">Nobody found by that name</div>
                                ) : null}
                            </div>
                        </div>
                    ) : activeKey && activeRow ? (
                        <>
                            {/* Conversation header, 78 px: who (live ring, badges, painted
                                name), whether they are live, then Watch and More. The second
                                line is always present (live line, else @login), so the name
                                never shifts when they go live or end. */}
                            <div className="flex h-[78px] shrink-0 items-center gap-3.5 border-b border-borderSubtle px-[22px]">
                                <button
                                    type="button"
                                    onClick={handleProfileClick}
                                    aria-label={`Open ${activeRow.user_name}'s profile`}
                                    className="-ml-1.5 flex min-w-0 flex-1 items-center gap-3.5 rounded-xl p-1.5 text-left transition-colors hover:bg-white/[0.04]"
                                >
                                    <Face row={activeRow} size={48} />
                                    <span className="flex min-w-0 flex-col gap-1">
                                        <span className="flex min-w-0 items-center gap-1.5 text-[18px] leading-[22px] text-textPrimary">
                                            <PinnedChatter
                                                provider="twitch"
                                                userId={activeRow.user_id}
                                                name={activeRow.user_name}
                                                login={activeRow.user_login}
                                                color={activeRow.color ?? undefined}
                                                badges={activeRow.badges}
                                                variant="line"
                                                fold
                                                nameClassName="min-w-0 shrink-0 max-w-full truncate font-bold"
                                            />
                                            {live && <span className="shrink-0 text-[13px] font-normal text-textMuted">@{activeRow.user_login}</span>}
                                        </span>
                                        {live ? (
                                            <span className="flex min-w-0 items-center gap-2 text-[12.5px] leading-4 text-textSecondary">
                                                <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-live" aria-hidden />
                                                <span className="truncate">{live}</span>
                                            </span>
                                        ) : (
                                            <span className="truncate text-[12.5px] leading-4 text-textMuted">@{activeRow.user_login}</span>
                                        )}
                                    </span>
                                </button>
                                <div className="flex shrink-0 items-center gap-3.5 self-start mt-[10px]">
                                {activeRow.live && (
                                    <button type="button" onClick={watchLive} className={`${GLAZE_PILL} flex h-9 shrink-0 items-center gap-2 px-3.5 text-[13.5px] font-semibold`}>
                                        <Play size={15} fill="currentColor" aria-hidden />
                                        Watch
                                    </button>
                                )}
                                <div className="relative shrink-0" ref={moreRef} onPointerDownCapture={markInsideMore}>
                                    <button
                                        type="button"
                                        aria-label="More"
                                        aria-expanded={showMore}
                                        onClick={() => setShowMore((v) => !v)}
                                        className={`${GLAZE_PILL} flex h-9 w-9 items-center justify-center !p-0`}
                                    >
                                        <MoreHorizontal size={18} />
                                    </button>
                                    {showMore && (
                                        <div className="glass-flyout no-live-blur absolute right-0 top-full z-10 mt-1.5 min-w-[160px] rounded-xl">
                                            <button
                                                type="button"
                                                onClick={() => { setShowMore(false); handleProfileClick(); }}
                                                className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-[13px] text-textPrimary transition-colors hover:bg-white/[0.06]"
                                            >
                                                <User size={14} className="text-textSecondary" /> Profile
                                            </button>
                                            <button
                                                type="button"
                                                onClick={() => { setShowMore(false); archiveConversation(activeRow); }}
                                                className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-[13px] text-textPrimary transition-colors hover:bg-white/[0.06]"
                                            >
                                                <Archive size={14} className="text-textSecondary" /> Archive
                                            </button>
                                        </div>
                                    )}
                                </div>
                                </div>
                            </div>

                            {/* Messages: glass bubbles, newest at the bottom */}
                            <div ref={scrollRef} onScroll={onMessagesScroll} className="min-h-0 flex-1 overflow-y-auto px-[22px] py-[18px] scrollbar-thin" style={{ maskImage: 'linear-gradient(to bottom, transparent, black 16px, black calc(100% - 12px), transparent)', WebkitMaskImage: 'linear-gradient(to bottom, transparent, black 16px, black calc(100% - 12px), transparent)' }}>
                                <div className="flex min-h-full flex-col justify-end">
                                    {page && !page.hasMoreLocal && (page.olderOnTwitch || olderNote) && (
                                        <div className="mb-3 flex min-h-8 items-center justify-center gap-2 text-[12px] text-textMuted">
                                            {loadingOlder ? (
                                                <Loader2 size={14} className={instant ? '' : 'animate-spin'} />
                                            ) : (
                                                <>
                                                    <span>{olderNote ?? 'Older messages are on Twitch'}</span>
                                                    {page.olderOnTwitch && (
                                                        <button type="button" onClick={loadFromTwitch} className={`${GLAZE_PILL} h-7 px-3 text-[12px]`}>
                                                            Load older
                                                        </button>
                                                    )}
                                                </>
                                            )}
                                        </div>
                                    )}
                                    {page && page.hasMoreLocal && loadingOlder && (
                                        <div className="mb-3 flex min-h-8 justify-center">
                                            <Loader2 size={14} className={`text-textMuted ${instant ? '' : 'animate-spin'}`} />
                                        </div>
                                    )}
                                    {!page ? (
                                        <div className="flex flex-1 items-center justify-center">
                                            <Loader2 size={20} className={`text-textMuted ${instant ? '' : 'animate-spin'}`} />
                                        </div>
                                    ) : page.messages.length === 0 ? (
                                        <div className="flex flex-1 flex-col items-center justify-center text-textMuted">
                                            <MessageCircle size={40} className="mb-3 text-accent/50 opacity-50" />
                                            <span className="text-sm">No messages yet</span>
                                            <span className="mt-1 text-xs">Say hi to {activeRow.user_name}</span>
                                        </div>
                                    ) : (
                                        <WhisperMessages
                                            messages={page.messages}
                                            nowMs={now}
                                            formatTime={(ms) => messageTime(ms, now)}
                                            instant={instant}
                                            onRetry={retry}
                                            onDiscard={discard}
                                        />
                                    )}
                                </div>
                            </div>

                            {/* Composer: an error strip above it when a send was refused
                                before Rust filed it, then the glazed bar. */}
                            <div className="relative shrink-0 px-[22px] pb-[18px]">
                                {error && (
                                    <div className={COMPOSER_STRIP} style={COMPOSER_STRIP_STYLE} role="alert">
                                        <span className="min-w-0 flex-1 text-[12.5px] leading-[18px] text-error">{error}</span>
                                        <button type="button" onClick={() => setError(null)} aria-label="Dismiss" className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-textMuted hover:text-textPrimary">
                                            <X size={14} />
                                        </button>
                                    </div>
                                )}
                                {showEmojiPicker && (
                                    <div ref={emojiPickerRef} onPointerDownCapture={markInsideEmojiPicker} className="absolute bottom-full left-[22px] right-[22px] mb-2 glass-flyout no-live-blur rounded-xl !p-0 overflow-hidden" style={{ height: '320px' }}>
                                        <div className="p-3 border-b border-white/[0.06]">
                                            <input
                                                type="text"
                                                value={emojiSearchQuery}
                                                onChange={(e) => setEmojiSearchQuery(e.target.value)}
                                                placeholder="Search emojis..."
                                                className="w-full glass-input rounded-lg px-3 py-2 text-xs transition-colors"
                                            />
                                            <div className="flex gap-1 mt-2 overflow-x-auto scrollbar-thin pb-1">
                                                {Object.keys(emojiCategories).map((category) => (
                                                    <button
                                                        type="button"
                                                        key={category}
                                                        onClick={() => { setSelectedEmojiCategory(category); setEmojiSearchQuery(''); }}
                                                        className={`px-3 py-1.5 whitespace-nowrap ${selectedEmojiCategory === category ? `${ACCENT_BUTTON} !text-xs` : `${GLAZE_PILL} text-xs`}`}
                                                        style={selectedEmojiCategory === category ? ACCENT_FILL : undefined}
                                                    >
                                                        {category}
                                                    </button>
                                                ))}
                                            </div>
                                        </div>
                                        <div className="p-3 overflow-y-auto scrollbar-thin" style={{ height: 'calc(100% - 90px)' }}>
                                            <div className="grid grid-cols-8 gap-1">
                                                {getFilteredEmojis().map((emoji, idx) => (
                                                    <Tooltip key={`${emoji}-${idx}`} content={emoji} side="top">
                                                        <button
                                                            type="button"
                                                            onClick={() => insertEmoji(emoji)}
                                                            className="flex items-center justify-center p-2 hover:bg-white/[0.06] rounded-lg transition-colors"
                                                        >
                                                            <img src={getAppleEmojiUrl(emoji)} alt={emoji} className="w-6 h-6 object-contain" onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; (e.target as HTMLImageElement).insertAdjacentText('afterend', emoji); }} />
                                                        </button>
                                                    </Tooltip>
                                                ))}
                                            </div>
                                        </div>
                                    </div>
                                )}

                                <div className={`${GLAZE_BANNER} relative flex items-end gap-2.5 py-2 pl-3 pr-2`} style={COMPOSER_BAR_STYLE}>
                                    <button
                                        type="button"
                                        aria-label="Emoji"
                                        ref={emojiButtonRef}
                                        onClick={() => setShowEmojiPicker(!showEmojiPicker)}
                                        className={`flex h-[38px] w-[38px] shrink-0 items-center justify-center rounded-xl transition-colors ${
                                            showEmojiPicker ? 'bg-accent/10 text-accent' : 'text-textSecondary hover:bg-white/[0.06] hover:text-textPrimary'
                                        }`}
                                    >
                                        <Smile size={20} />
                                    </button>
                                    {/* Wrapper so the spell-check underlay can sit over the
                                        textarea; the textarea keeps the `flex-1` sizing. */}
                                    <div className="relative flex min-w-0 flex-1">
                                        <textarea
                                            ref={inputRef}
                                            value={message}
                                            onChange={(e) => setMessage(e.target.value)}
                                            onKeyDown={handleKeyDown}
                                            onFocus={warmSpellcheck}
                                            spellCheck={false}
                                            data-spellcheck={spellcheckEnabled ? 'true' : undefined}
                                            placeholder={`Whisper ${activeRow.user_name}`}
                                            aria-label={`Whisper ${activeRow.user_name}`}
                                            rows={1}
                                            className="min-h-[38px] max-h-[120px] w-full resize-none bg-transparent py-[8.5px] text-[14.5px] leading-[21px] text-textPrimary outline-none placeholder:text-textMuted"
                                            style={{ height: 'auto' }}
                                            onScroll={(e) => {
                                                if (spellUnderlayRef.current) {
                                                    spellUnderlayRef.current.scrollTop = e.currentTarget.scrollTop;
                                                }
                                            }}
                                            onInput={(e) => {
                                                const target = e.target as HTMLTextAreaElement;
                                                target.style.height = 'auto';
                                                target.style.height = Math.min(target.scrollHeight, 120) + 'px';
                                            }}
                                        />
                                        {spellRanges.length > 0 && (
                                            <SpellcheckUnderlay
                                                innerRef={spellUnderlayRef}
                                                text={message}
                                                ranges={spellRanges}
                                                className="py-[8.5px] text-[14.5px] leading-[21px]"
                                            />
                                        )}
                                    </div>
                                    {/* The counter's slot sits beside Send from 450 on; it never
                                        changes the bar's height. */}
                                    {counter && (
                                        <span className={`shrink-0 self-center text-[12px] tabular-nums ${over ? 'text-error' : 'text-textMuted'}`}>{counter}</span>
                                    )}
                                    <button
                                        type="button"
                                        aria-label="Send"
                                        onClick={() => void handleSend()}
                                        disabled={!message.trim() || isSending || over}
                                        className={`${ACCENT_BUTTON} flex h-[38px] w-[38px] shrink-0 items-center justify-center !p-0 disabled:opacity-40 disabled:cursor-not-allowed`}
                                        style={ACCENT_FILL}
                                    >
                                        {isSending ? <Loader2 size={18} className={instant ? '' : 'animate-spin'} /> : <Send size={18} />}
                                    </button>
                                </div>
                            </div>
                        </>
                    ) : (
                        <div className="flex min-h-0 flex-1 flex-col items-center justify-center p-8 text-textMuted">
                            <MessageCircle size={56} className="mb-4 text-accent opacity-30" />
                            <span className="mb-6 text-lg font-semibold text-textPrimary">Pick a conversation</span>
                            <button
                                type="button"
                                onClick={() => setShowNewConversation(true)}
                                disabled={signedOut}
                                className={`${ACCENT_BUTTON} h-10 px-5 disabled:opacity-50`}
                                style={ACCENT_FILL}
                            >
                                New whisper
                            </button>
                        </div>
                    )}
                </div>
                </div>
            </motion.div>
        </motion.div>
    );
};

export default WhispersWidget;
