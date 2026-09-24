import { useState, useEffect, useRef, useCallback } from 'react';
import { motion } from 'framer-motion';
import { X, MessageCircle, Send, Search, Plus, ArrowLeft, Loader2, Users, ChevronDown, Smile, SortAsc, User, Trash2, Clock, Download, RefreshCw } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { getAppleEmojiUrl } from '../services/emojiService';
import WhisperImportWizard from './WhisperImportWizard';
import { Tooltip } from './ui/Tooltip';
import SpellcheckUnderlay from './chat/SpellcheckUnderlay';
import { useSpellcheck } from '../hooks/useSpellcheck';
import { warmSpellcheck } from '../utils/spellcheck';
import type { WhisperConversation, Whisper, WhisperUpdate, UserInfo } from '../types';

import { Logger } from '../utils/logger';
import { playNotificationSound as playGatedNotificationSound } from '../utils/notificationSound';
interface WhispersWidgetProps {
    isOpen: boolean;
    onClose: () => void;
}

type SortOption = 'recent' | 'name' | 'unread';

const WHISPERS_LOCALSTORAGE_KEY = 'streamnook-whisper-conversations';
const WHISPERS_MIGRATED_KEY = 'streamnook-whispers-migrated-to-disk';

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

// Helper to convert Map to object for Tauri storage
const mapToObject = (map: Map<string, WhisperConversation>): Record<string, WhisperConversation> => {
    const obj: Record<string, WhisperConversation> = {};
    map.forEach((value, key) => {
        obj[key] = value;
    });
    return obj;
};

// Helper to convert object from Tauri storage to Map
const objectToMap = (obj: Record<string, WhisperConversation>): Map<string, WhisperConversation> => {
    const map = new Map<string, WhisperConversation>();
    Object.entries(obj).forEach(([key, value]) => {
        map.set(key, value);
    });
    return map;
};

// Load conversations from disk via Tauri backend (scoped to the owning account)
const loadConversationsFromDisk = async (ownerId: string): Promise<Map<string, WhisperConversation>> => {
    try {
        const storage = await invoke<{ conversations: Record<string, WhisperConversation>; version: number }>('load_whisper_storage', { ownerId });
        if (storage && storage.conversations) {
            const map = objectToMap(storage.conversations);
            Logger.debug('[Whispers] Loaded from disk:', map.size, 'conversations');
            return map;
        }
    } catch (error) {
        Logger.warn('[Whispers] Failed to load from disk:', error);
    }
    return new Map();
};

// Legacy: Load from localStorage for migration purposes
const loadConversationsFromLocalStorage = (): Map<string, WhisperConversation> | null => {
    try {
        const data = localStorage.getItem(WHISPERS_LOCALSTORAGE_KEY);
        if (data) {
            const entries = JSON.parse(data) as [string, WhisperConversation][];
            // Deduplicate messages in each conversation
            const deduplicatedEntries = entries.map(([userId, conv]) => {
                const seenIds = new Set<string>();
                const uniqueMessages = conv.messages.filter(msg => {
                    if (seenIds.has(msg.id)) {
                        return false;
                    }
                    seenIds.add(msg.id);
                    return true;
                });
                return [userId, { ...conv, messages: uniqueMessages }] as [string, WhisperConversation];
            });
            return new Map(deduplicatedEntries);
        }
    } catch (error) {
        Logger.warn('[Whispers] Failed to load from localStorage:', error);
    }
    return null;
};

// Migrate from localStorage to disk storage (one-time migration), into the
// owning account's scoped storage.
const migrateFromLocalStorage = async (ownerId: string): Promise<Map<string, WhisperConversation>> => {
    // Check if already migrated
    if (localStorage.getItem(WHISPERS_MIGRATED_KEY)) {
        Logger.debug('[Whispers] Already migrated, loading from disk');
        return loadConversationsFromDisk(ownerId);
    }

    // Check if there's localStorage data to migrate
    const localStorageData = loadConversationsFromLocalStorage();
    if (localStorageData && localStorageData.size > 0) {
        Logger.debug('[Whispers] Migrating', localStorageData.size, 'conversations from localStorage to disk...');
        try {
            const conversationsObj = mapToObject(localStorageData);
            await invoke('migrate_whispers_from_localstorage', { ownerId, conversations: conversationsObj });
            // Mark as migrated
            localStorage.setItem(WHISPERS_MIGRATED_KEY, 'true');
            // Clear localStorage data (optional - keep for safety)
            // localStorage.removeItem(WHISPERS_LOCALSTORAGE_KEY);
            Logger.debug('[Whispers] Migration complete');
            return localStorageData;
        } catch (error) {
            Logger.error('[Whispers] Migration failed:', error);
            // Fall back to localStorage data
            return localStorageData;
        }
    }

    // No localStorage data, mark as migrated and load from disk
    localStorage.setItem(WHISPERS_MIGRATED_KEY, 'true');
    return loadConversationsFromDisk(ownerId);
};

/** Fold one Rust-side change into the panel's map. A conversation that moved
 *  from a login key to an id carries its messages over; Rust keeps messages in
 *  time order, and so does this. */
const applyWhisperUpdate = (
    prev: Map<string, WhisperConversation>,
    update: WhisperUpdate,
): Map<string, WhisperConversation> => {
    const next = new Map(prev);
    const moved = update.replaced_key ? next.get(update.replaced_key) : undefined;
    if (update.replaced_key) next.delete(update.replaced_key);
    const current = next.get(update.key);

    const byId = new Map<string, Whisper>();
    for (const m of [...(current?.messages ?? []), ...(moved?.messages ?? [])]) byId.set(m.id, m);
    if (update.message) byId.set(update.message.id, update.message);
    const messages = [...byId.values()].sort((a, b) => a.timestamp - b.timestamp);

    next.set(update.key, {
        ...update.meta,
        profile_image_url: update.meta.profile_image_url ?? undefined,
        messages,
    });
    return next;
};

const WhispersWidget = ({ isOpen, onClose }: WhispersWidgetProps) => {
    const [conversations, setConversations] = useState<Map<string, WhisperConversation>>(new Map());
    const [activeConversation, setActiveConversation] = useState<string | null>(null);
    const { whisperTargetUser, clearWhisperTargetUser, whisperImportState } = useAppStore();
    const previousActiveConversationRef = useRef<string | null>(null);
    const [message, setMessage] = useState('');
    const [isSending, setIsSending] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [searchQuery, setSearchQuery] = useState('');
    const [isSearching, setIsSearching] = useState(false);
    const [searchResults, setSearchResults] = useState<UserInfo[]>([]);
    const [showNewConversation, setShowNewConversation] = useState(false);
    const [isImportingAll, setIsImportingAll] = useState(false);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [importProgress, setImportProgress] = useState<string>('');
    const [sortOption, setSortOption] = useState<SortOption>('recent');
    const [showSortMenu, setShowSortMenu] = useState(false);
    const [showEmojiPicker, setShowEmojiPicker] = useState(false);
    const [emojiSearchQuery, setEmojiSearchQuery] = useState('');
    const [selectedEmojiCategory, setSelectedEmojiCategory] = useState<string>('Smileys');
    const [showImportWizard, setShowImportWizard] = useState(false);

    const messagesEndRef = useRef<HTMLDivElement>(null);
    const inputRef = useRef<HTMLTextAreaElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const sortMenuRef = useRef<HTMLDivElement>(null);
    const emojiPickerRef = useRef<HTMLDivElement>(null);
    const sendingRef = useRef(false);

    const { currentUser, settings, setProfileModalUser } = useAppStore();

    // Spell check for the whisper composer. No channel here, so there's no emote
    // set to consult — only the custom dictionary and the chatter list apply.
    const spellcheckEnabled = settings.chat_input?.spellcheck_enabled ?? true;
    const spellUnderlayRef = useRef<HTMLDivElement>(null);
    const spellRanges = useSpellcheck(message, { enabled: spellcheckEnabled, emoteKey: null });

    // Many conversations changed at once in Rust (an import or a refresh):
    // reload the archive, keeping any draft conversation that has no messages
    // yet (those exist only here until their first send).
    const reloadConversations = useCallback(async () => {
        const ownerId = currentUser?.user_id;
        if (!ownerId) return;
        const loaded = await loadConversationsFromDisk(ownerId);
        setConversations((prev) => {
            for (const [key, conv] of prev) {
                if (conv.messages.length === 0 && !loaded.has(key)) loaded.set(key, conv);
            }
            return loaded;
        });
    }, [currentUser?.user_id]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        let isMounted = true;
        void listen<string>('whisper-archive-changed', (event) => {
            if (event.payload === currentUser?.user_id) void reloadConversations();
        }).then((fn) => {
            if (isMounted) unlisten = fn;
            else fn();
        });
        return () => {
            isMounted = false;
            unlisten?.();
        };
    }, [currentUser?.user_id, reloadConversations]);

    // Handle initial target user from profile card whisper button
    useEffect(() => {
        if (isOpen && whisperTargetUser) {
            // Create or select conversation for the target user
            const userId = whisperTargetUser.id;
            if (!conversations.has(userId)) {
                setConversations(prev => {
                    const n = new Map(prev);
                    n.set(userId, {
                        user_id: userId,
                        user_login: whisperTargetUser.login,
                        user_name: whisperTargetUser.display_name,
                        profile_image_url: whisperTargetUser.profile_image_url ?? undefined,
                        messages: [],
                        last_message_timestamp: Date.now(),
                        unread_count: 0
                    });
                    return n;
                });
            }
            setActiveConversation(userId);
            setShowNewConversation(false);
            // Clear the target user after handling
            clearWhisperTargetUser();
        }
    }, [isOpen, whisperTargetUser, conversations, clearWhisperTargetUser]);

    // Get current conversation
    const currentConversation = activeConversation ? conversations.get(activeConversation) : null;

    // Load conversations for the signed-in account (with one-time localStorage
    // migration). Whispers are per-account: signed out shows nothing, and
    // switching accounts loads that account's own whispers, never the previous
    // account's. Re-runs whenever the active account changes.
    useEffect(() => {
        const ownerId = currentUser?.user_id;
        // Account changed (or signed out): drop the previous account's selection
        // so we never show a thread that isn't this account's.
        setActiveConversation(null);
        if (!ownerId) {
            setConversations(new Map());
            return;
        }
        let cancelled = false;
        const loadConversations = async () => {
            try {
                const loaded = await migrateFromLocalStorage(ownerId);
                if (!cancelled) setConversations(loaded);
            } catch (error) {
                Logger.error('[Whispers] Failed to load conversations:', error);
            }
        };
        loadConversations();
        return () => { cancelled = true; };
    }, [currentUser?.user_id]);

    // Pull the latest messages for existing conversations straight from Twitch
    // (web session token + whisper GQL) and merge anything new into history by
    // timestamp — e.g. replies you sent from the Twitch site or your phone that
    // StreamNook never saw. Cheap: one recent page per conversation.
    const handleRefresh = async () => {
        const ownerId = currentUser?.user_id;
        if (!ownerId || isRefreshing) return;
        if (conversations.size === 0) {
            setImportProgress('No conversations to refresh');
            setTimeout(() => setImportProgress(''), 2500);
            return;
        }
        setIsRefreshing(true);
        setImportProgress('Refreshing messages…');
        try {
            const added = await invoke<number>('whisper_refresh', { ownerId });
            setImportProgress(added > 0 ? `✓ Added ${added} new message${added === 1 ? '' : 's'}` : '✓ Up to date');
            setTimeout(() => setImportProgress(''), 3000);
        } catch (e) {
            Logger.error('[Whispers] Refresh failed:', e);
            setImportProgress('Refresh failed');
            setTimeout(() => setImportProgress(''), 3000);
        } finally {
            setIsRefreshing(false);
        }
    };

    // Close menus when clicking outside
    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (sortMenuRef.current && !sortMenuRef.current.contains(event.target as Node)) {
                setShowSortMenu(false);
            }
            if (emojiPickerRef.current && !emojiPickerRef.current.contains(event.target as Node)) {
                setShowEmojiPicker(false);
            }
        };
        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, []);

    // Play notification sound. Shared gate, so a whisper that also raises a
    // notification-center entry pings once rather than twice.
    const playNotificationSound = useCallback(() => {
        playGatedNotificationSound('whisper');
    }, []);

    // Whispers are recorded by Rust as they arrive (sent ones too); each change
    // lands here as the one conversation it touched.
    useEffect(() => {
        let isMounted = true;
        let unlistenFn: (() => void) | undefined;
        void listen<WhisperUpdate>('whisper-conversation-updated', (event) => {
            const update = event.payload;
            if (update.owner_id !== currentUser?.user_id) return;
            if (update.message && !update.message.is_sent && settings.live_notifications?.play_sound) {
                playNotificationSound();
            }
            setConversations((prev) => applyWhisperUpdate(prev, update));
            if (update.replaced_key) {
                setActiveConversation((active) => (active === update.replaced_key ? update.key : active));
            }
        }).then((fn) => {
            if (isMounted) unlistenFn = fn;
            else fn();
        });
        return () => {
            isMounted = false;
            unlistenFn?.();
        };
    }, [currentUser?.user_id, playNotificationSound, settings.live_notifications?.play_sound]);

    // Tell Rust which conversation is open, so a whisper arriving in it is not
    // counted as unread.
    useEffect(() => {
        void invoke('whisper_set_active', { key: activeConversation });
    }, [activeConversation]);
    useEffect(() => () => void invoke('whisper_set_active', { key: null }), []);

    // Scroll to bottom when new messages arrive
    useEffect(() => {
        if (activeConversation) {
            const container = messagesEndRef.current?.closest('.overflow-y-auto');
            if (container) {
                container.scrollTo({ top: container.scrollHeight, behavior: 'smooth' });
            }
        }
    }, [conversations, activeConversation]);

    // Focus input when opening a conversation
    useEffect(() => {
        if (activeConversation) {
            inputRef.current?.focus({ preventScroll: true });
        }
    }, [activeConversation]);

    // Mark conversation as read when viewing and clean up empty conversations when switching away
    useEffect(() => {
        // Clean up the previous conversation if it was empty
        if (previousActiveConversationRef.current && previousActiveConversationRef.current !== activeConversation) {
            const prevConvId = previousActiveConversationRef.current;
            setConversations(prev => {
                const prevConv = prev.get(prevConvId);
                if (prevConv && prevConv.messages.length === 0) {
                    // Remove empty conversation
                    const newConversations = new Map(prev);
                    newConversations.delete(prevConvId);
                    Logger.debug('[Whispers] Removed empty conversation:', prevConvId);
                    return newConversations;
                }
                return prev;
            });
        }

        // Update the ref to current
        previousActiveConversationRef.current = activeConversation;

        // Mark current conversation as read. Rust only writes when the count
        // actually changes, so opening an already-read thread costs nothing.
        if (activeConversation) {
            const ownerId = useAppStore.getState().currentUser?.user_id;
            if (ownerId) {
                invoke('whisper_mark_read', { ownerId, key: activeConversation }).catch((err) =>
                    Logger.warn('[Whispers] Failed to mark read:', err),
                );
            }
            setConversations(prev => {
                const existing = prev.get(activeConversation);
                if (!existing || existing.unread_count === 0) return prev;
                const newConversations = new Map(prev);
                newConversations.set(activeConversation, { ...existing, unread_count: 0 });
                return newConversations;
            });
        }
    }, [activeConversation]);

    // Clean up empty conversations when closing the widget
    const handleClose = useCallback(() => {
        // Clean up the current conversation if it's empty
        if (activeConversation) {
            setConversations(prev => {
                const conv = prev.get(activeConversation);
                if (conv && conv.messages.length === 0) {
                    const newConversations = new Map(prev);
                    newConversations.delete(activeConversation);
                    Logger.debug('[Whispers] Removed empty conversation on close:', activeConversation);
                    return newConversations;
                }
                return prev;
            });
        }
        setActiveConversation(null);
        onClose();
    }, [activeConversation, onClose]);

    // Import whispers from exported JSON file
    const handleFileImport = async (event: React.ChangeEvent<HTMLInputElement>) => {
        const file = event.target.files?.[0];
        const ownerId = currentUser?.user_id;
        if (!file || !ownerId) return;
        setIsImportingAll(true);
        setImportProgress('Reading file...');
        try {
            const data = JSON.parse(await file.text());
            if (!data.version || !data.conversations || !Array.isArray(data.conversations)) {
                throw new Error('Invalid file format.');
            }
            setImportProgress(`Importing ${data.conversations.length} conversations...`);
            const summary = await invoke<{ conversations: number; messages: number }>('whisper_import', {
                ownerId,
                export: data,
            });
            setImportProgress(`✓ Imported ${summary.messages} messages from ${summary.conversations} users`);
            if (fileInputRef.current) fileInputRef.current.value = '';
            setTimeout(() => setImportProgress(''), 3000);
        } catch (err) {
            Logger.error('[Whispers] Failed to import file:', err);
            setError(err instanceof Error ? err.message : 'Failed to import');
            setImportProgress('');
        } finally {
            setIsImportingAll(false);
        }
    };

    // Search for users
    const handleSearch = async () => {
        if (!searchQuery.trim()) { setSearchResults([]); return; }
        setIsSearching(true);
        try {
            try {
                const whisperSearchResult = await invoke<[string, string, string, string | null] | null>('search_whisper_user', { username: searchQuery });
                if (whisperSearchResult) {
                    const [userId, userLogin, displayName, profileImageUrl] = whisperSearchResult;
                    setSearchResults([{ id: userId, login: userLogin, display_name: displayName, profile_image_url: profileImageUrl || undefined }]);
                    return;
                }
            } catch { /* fallback */ }
            const results = await invoke<Record<string, unknown>[]>('search_channels', { query: searchQuery });
            setSearchResults(results.slice(0, 10).map((r: Record<string, unknown>) => ({ id: (r.user_id || r.id) as string, login: (r.user_login || r.login) as string, display_name: (r.user_name || r.display_name) as string, profile_image_url: r.profile_image_url as string | undefined })).filter(r => r.id && r.login));
        } catch { setSearchResults([]); }
        finally { setIsSearching(false); }
    };

    // Start conversation with user
    const startConversation = async (user: UserInfo) => {
        if (!conversations.has(user.id)) {
            setConversations(prev => {
                const n = new Map(prev);
                n.set(user.id, { user_id: user.id, user_login: user.login, user_name: user.display_name, profile_image_url: user.profile_image_url, messages: [], last_message_timestamp: Date.now(), unread_count: 0 });
                return n;
            });
        }
        setActiveConversation(user.id);
        setShowNewConversation(false);
        setSearchQuery('');
        setSearchResults([]);
    };

    const handleSend = async () => {
        if (!message.trim() || isSending || !activeConversation || sendingRef.current) return;
        const conversation = conversations.get(activeConversation);
        if (!conversation) return;

        const ownerId = currentUser?.user_id;
        if (!ownerId) return;

        const messageToSend = message.trim();
        sendingRef.current = true;
        setMessage('');
        setIsSending(true);
        setError(null);
        setShowEmojiPicker(false);
        try {
            const update = await invoke<WhisperUpdate>('whisper_send', {
                ownerId,
                target: {
                    key: activeConversation,
                    user_id: conversation.user_id,
                    user_login: conversation.user_login,
                    user_name: conversation.user_name,
                    profile_image_url: conversation.profile_image_url ?? null,
                },
                text: messageToSend,
            });
            setConversations((prev) => applyWhisperUpdate(prev, update));
        } catch (err) {
            Logger.error('Failed to send whisper:', err);

            // Parse the error to provide a more specific message
            const errorString = String(err);
            let errorMessage = 'Failed to send message. Please try again.';

            // Check for specific Twitch API errors
            if (errorString.includes('recipient\'s settings prevent') ||
                errorString.includes('recipient\\"s settings prevent') ||
                errorString.includes('settings prevent this sender from whispering')) {
                errorMessage = 'Cannot send: This user\'s privacy settings prevent them from receiving whispers from you.';
            } else if (errorString.includes('403') || errorString.includes('Forbidden')) {
                errorMessage = 'Cannot send: You don\'t have permission to whisper this user.';
            } else if (errorString.includes('401') || errorString.includes('Unauthorized')) {
                errorMessage = 'Cannot send: Your session has expired. Please log in again.';
            } else if (errorString.includes('429') || errorString.includes('Too Many Requests')) {
                errorMessage = 'Sending too fast. Please wait a moment before trying again.';
            } else if (errorString.includes('404') || errorString.includes('Not Found')) {
                errorMessage = 'Cannot send: User not found.';
            }

            setError(errorMessage);
            setMessage(messageToSend);
        } finally {
            setIsSending(false);
            sendingRef.current = false;
            inputRef.current?.focus({ preventScroll: true });
        }
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
    };

    const insertEmoji = (emoji: string) => {
        setMessage(prev => prev + emoji);
        inputRef.current?.focus({ preventScroll: true });
    };

    const formatTime = (timestamp: number) => {
        const date = new Date(timestamp);
        const now = new Date();
        const isToday = date.toDateString() === now.toDateString();
        if (isToday) return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
        return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
    };

    // Get sorted conversations with their Map keys
    const sortedConversations = Array.from(conversations.entries())
        .map(([key, conv]) => ({ key, ...conv }))
        .sort((a, b) => {
            if (sortOption === 'recent') return b.last_message_timestamp - a.last_message_timestamp;
            if (sortOption === 'name') return a.user_name.localeCompare(b.user_name);
            if (sortOption === 'unread') return b.unread_count - a.unread_count || b.last_message_timestamp - a.last_message_timestamp;
            return 0;
        });

    const totalUnread = sortedConversations.reduce((sum, c) => sum + c.unread_count, 0);

    // Get filtered emojis
    const getFilteredEmojis = () => {
        const categoryEmojis = emojiCategories[selectedEmojiCategory] || [];
        if (!emojiSearchQuery) return categoryEmojis;
        const allEmojis = Object.values(emojiCategories).flat();
        return allEmojis.filter(emoji => emoji.includes(emojiSearchQuery));
    };

    const parseTextWithLinks = (text: string) => {
        const urlRegex = /(https?:\/\/[^\s]+|www\.[^\s]+)/g;
        const parts = text.split(urlRegex);

        return parts.map((part, index) => {
            if (part.match(/^(https?:\/\/|www\.)/)) {
                const url = part.startsWith('http') ? part : `https://${part}`;
                return (
                    <a
                        key={index}
                        href={url}
                        // No target="_blank": the webview opens _blank links externally on
                        // its own, which would stack a second tab on top of the open() below.
                        className="text-accent hover:text-accentHover underline cursor-pointer"
                        onClick={async (e) => {
                            e.preventDefault();
                            try {
                                const { open } = await import('@tauri-apps/plugin-shell');
                                await open(url);
                            } catch (err) {
                                Logger.error('[Whispers] Failed to open URL:', err);
                            }
                        }}
                    >
                        {part}
                    </a>
                );
            }
            return part;
        });
    };

    const handleProfileClick = () => {
        if (!currentConversation) return;

        // Ensure we're not causing recursive re-renders by wrapping in a stable callback
        setProfileModalUser({
            id: currentConversation.user_id,
            user_id: currentConversation.user_id,
            user_name: currentConversation.user_name,
            user_login: currentConversation.user_login,
            title: `${currentConversation.user_name}'s Profile`,
            viewer_count: 0,
            game_name: '',
            thumbnail_url: currentConversation.profile_image_url || '',
            started_at: '',
            profile_image_url: currentConversation.profile_image_url || '',
        });
    };

    if (!isOpen) return null;

    return (
        <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-md"
            onClick={handleClose}
        >
            <motion.div
                initial={{ scale: 0.95, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                exit={{ scale: 0.95, opacity: 0 }}
                transition={{ type: 'spring', stiffness: 300, damping: 30 }}
                className="glass-panel w-[700px] h-[650px] flex overflow-hidden shadow-[0_20px_50px_rgba(0,0,0,0.5)] bg-surface/50 !backdrop-blur-3xl"
                onClick={(e) => e.stopPropagation()}
            >
                {/* Sidebar - Conversation List */}
                <div className={`w-full md:w-[280px] flex-shrink-0 border-r border-borderSubtle flex flex-col min-h-0 ${activeConversation && 'hidden md:flex'}`}>
                    {/* Header */}
                    <div className="flex items-center justify-between px-4 py-4 border-b border-borderSubtle shrink-0">
                        <div className="flex items-center gap-2">
                            <MessageCircle size={20} className="text-accent" />
                            <span className="text-textPrimary font-semibold text-lg hover:text-accent transition-colors">Whispers</span>
                            {totalUnread > 0 && (
                                <span className="glass-badge text-textPrimary text-xs px-2 py-0.5 rounded-full font-medium">
                                    {totalUnread}
                                </span>
                            )}
                        </div>
                        <div className="flex items-center gap-1 shrink-0">
                            <input type="file" ref={fileInputRef} onChange={handleFileImport} accept=".json" className="hidden" />
                            {/* Background Import Indicator */}
                            {whisperImportState.isImporting && (
                                <Tooltip content="Click to view import progress" side="bottom">
                                <button
                                    onClick={() => setShowImportWizard(true)}
                                    className="flex items-center gap-2 px-2 py-1.5 glass-button hover:text-accent transition-colors animate-pulse"
                                >
                                    <Loader2 size={14} className="text-accent animate-spin" />
                                    <span className="text-accent text-xs font-medium">
                                        {whisperImportState.exportProgress.total > 0
                                            ? `${whisperImportState.exportProgress.current + 1}/${whisperImportState.exportProgress.total}`
                                            : 'Importing...'}
                                    </span>
                                </button>
                                </Tooltip>
                            )}
                            {/* Refresh: pull the latest messages from Twitch and merge them in */}
                            {!whisperImportState.isImporting && (
                                <Tooltip content="Refresh messages from Twitch" side="bottom">
                                <button
                                    onClick={handleRefresh}
                                    disabled={isRefreshing || isImportingAll}
                                    className="p-2 text-textSecondary hover:text-accent hover:bg-surface-hover rounded-lg transition-colors disabled:opacity-50"
                                >
                                    <RefreshCw size={18} className={isRefreshing ? 'animate-spin' : ''} />
                                </button>
                                </Tooltip>
                            )}
                            {/* Import Button (only show when not importing) */}
                            {!whisperImportState.isImporting && (
                                <Tooltip content="Import whisper history" side="bottom">
                                <button
                                    onClick={() => setShowImportWizard(true)}
                                    disabled={isImportingAll}
                                    className="p-2 text-textSecondary hover:text-accent hover:bg-surface-hover rounded-lg transition-colors disabled:opacity-50"
                                >
                                    {isImportingAll ? <Loader2 size={18} className="animate-spin" /> : <Download size={18} />}
                                </button>
                                </Tooltip>
                            )}
                            <Tooltip content="New conversation" side="bottom">
                            <button
                                onClick={() => setShowNewConversation(true)}
                                className="p-2 text-textSecondary hover:text-accent hover:bg-surface-hover rounded-lg transition-colors"
                            >
                                <Plus size={18} />
                            </button>
                            </Tooltip>
                            <button onClick={handleClose} className="p-2 text-textSecondary hover:text-error hover:bg-red-500/10 rounded-lg transition-colors md:hidden">
                                <X size={18} />
                            </button>
                        </div>
                    </div>

                    {/* Sort Options */}
                    <div className="px-4 py-2 border-b border-borderSubtle">
                        <div className="relative" ref={sortMenuRef}>
                            <button
                                onClick={() => setShowSortMenu(!showSortMenu)}
                                className="flex items-center gap-2 text-xs text-textSecondary hover:text-textPrimary transition-colors"
                            >
                                <SortAsc size={14} />
                                <span>Sort by: {sortOption === 'recent' ? 'Recent' : sortOption === 'name' ? 'Name' : 'Unread'}</span>
                                <ChevronDown size={14} className={`transition-transform ${showSortMenu ? 'rotate-180' : ''}`} />
                            </button>
                            {showSortMenu && (
                                <div className="absolute top-full left-0 mt-1 glass-panel z-10 py-1 min-w-[120px]">
                                    <button onClick={() => { setSortOption('recent'); setShowSortMenu(false); }} className={`w-full px-3 py-1.5 text-left text-xs hover:bg-surface-hover transition-colors flex items-center gap-2 ${sortOption === 'recent' ? 'text-accent font-medium' : 'text-textSecondary'}`}>
                                        <Clock size={12} /> Recent
                                    </button>
                                    <button onClick={() => { setSortOption('name'); setShowSortMenu(false); }} className={`w-full px-3 py-1.5 text-left text-xs hover:bg-surface-hover transition-colors flex items-center gap-2 ${sortOption === 'name' ? 'text-accent font-medium' : 'text-textSecondary'}`}>
                                        <User size={12} /> Name
                                    </button>
                                    <button onClick={() => { setSortOption('unread'); setShowSortMenu(false); }} className={`w-full px-3 py-1.5 text-left text-xs hover:bg-surface-hover transition-colors flex items-center gap-2 ${sortOption === 'unread' ? 'text-accent font-medium' : 'text-textSecondary'}`}>
                                        <MessageCircle size={12} /> Unread
                                    </button>
                                </div>
                            )}
                        </div>
                    </div>

                    {/* Import Progress */}
                    {importProgress && (
                        <div className="px-4 py-2 bg-accent/10 border-b border-accent/20">
                            <span className="text-accent text-xs">{importProgress}</span>
                        </div>
                    )}

                    {/* Conversation List */}
                    <div className="flex-1 overflow-y-auto min-h-0 scrollbar-thin">
                        {sortedConversations.length === 0 ? (
                            <div className="flex flex-col items-center justify-center h-full text-textMuted p-6">
                                <Users size={40} className="mb-3 opacity-50 text-accent/50" />
                                <span className="text-sm text-center mb-2">No conversations yet</span>
                                <button onClick={() => setShowNewConversation(true)} className="text-accent hover:text-accent font-medium text-sm transition-colors">
                                    Start a new conversation
                                </button>
                            </div>
                        ) : (
                            sortedConversations.map((conv) => (
                                <div
                                    key={conv.key}
                                    className={`group relative w-full flex items-center gap-3 px-4 py-3 hover:bg-surface-hover transition-colors cursor-pointer ${activeConversation === conv.key ? 'bg-surface-active' : ''}`}
                                    onClick={() => setActiveConversation(conv.key)}
                                >
                                    <div className="relative flex-shrink-0">
                                        {conv.profile_image_url ? (
                                            <img src={conv.profile_image_url} alt={conv.user_name} className="w-11 h-11 rounded-full object-cover border border-white/5" />
                                        ) : (
                                            <div className="w-11 h-11 rounded-full bg-accent/10 border border-accent/20 flex items-center justify-center">
                                                <MessageCircle size={18} className="text-accent" />
                                            </div>
                                        )}
                                        {conv.unread_count > 0 && (
                                            <div className="absolute -top-1 -right-1 w-5 h-5 glass-badge flex items-center justify-center text-textPrimary text-[10px] font-bold">
                                                {conv.unread_count > 9 ? '9+' : conv.unread_count}
                                            </div>
                                        )}
                                    </div>
                                    <div className="flex-1 min-w-0 text-left">
                                        <span className="text-textPrimary text-sm font-medium block truncate">{conv.user_name}</span>
                                        <span className="text-textMuted text-xs block truncate">
                                            {conv.messages.length > 0 ? conv.messages[conv.messages.length - 1].message : 'No messages'}
                                        </span>
                                    </div>
                                    <div className="flex items-center gap-2 flex-shrink-0">
                                        <span className="text-textMuted text-[10px] group-hover:hidden">{formatTime(conv.last_message_timestamp)}</span>
                                        <Tooltip content="Delete conversation" side="top">
                                        <button
                                            onClick={(e) => {
                                                e.stopPropagation();
                                                // Delete the conversation using the Map key
                                                const ownerId = currentUser?.user_id;
                                                if (ownerId) {
                                                    invoke('delete_whisper_conversation', { ownerId, userId: conv.key }).catch((err) =>
                                                        Logger.warn('[Whispers] Failed to delete conversation:', err),
                                                    );
                                                }
                                                setConversations(prev => {
                                                    const newConversations = new Map(prev);
                                                    newConversations.delete(conv.key);
                                                    return newConversations;
                                                });
                                                // If this was the active conversation, clear it
                                                if (activeConversation === conv.key) {
                                                    setActiveConversation(null);
                                                }
                                            }}
                                            className="hidden group-hover:flex p-1.5 text-textMuted hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                                        >
                                            <Trash2 size={14} />
                                        </button>
                                        </Tooltip>
                                    </div>
                                </div>
                            ))
                        )}
                    </div>
                </div>

                {/* Main Content */}
                <div className={`flex-1 flex flex-col min-w-0 ${!activeConversation && !showNewConversation && 'hidden md:flex'}`}>
                    {showNewConversation ? (
                        <div className="flex-1 flex flex-col min-h-0">
                            <div className="flex items-center gap-3 px-5 py-4 border-b border-borderSubtle shrink-0">
                                <button onClick={() => { setShowNewConversation(false); setSearchQuery(''); setSearchResults([]); }} className="p-2 text-textSecondary hover:text-accent hover:bg-surface-hover rounded-lg transition-colors">
                                    <ArrowLeft size={18} />
                                </button>
                                <span className="text-textPrimary font-semibold">New Conversation</span>
                            </div>
                            <div className="p-5 shrink-0">
                                <div className="flex items-center gap-3">
                                    <div className="flex-1 relative">
                                        <Search size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-textMuted" />
                                        <input
                                            type="text"
                                            value={searchQuery}
                                            onChange={(e) => setSearchQuery(e.target.value)}
                                            onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
                                            placeholder="Search for a user..."
                                            className="w-full glass-input rounded-lg pl-10 pr-4 py-2.5 text-sm transition-colors"
                                        />
                                    </div>
                                    <button onClick={handleSearch} disabled={isSearching} className="px-5 py-2.5 glass-button hover:text-accent disabled:opacity-50 text-textPrimary rounded-lg text-sm transition-colors">
                                        {isSearching ? <Loader2 size={16} className="animate-spin" /> : 'Search'}
                                    </button>
                                </div>
                            </div>
                            <div className="flex-1 overflow-y-auto min-h-0 px-5 pb-5">
                                {searchResults.length > 0 ? (
                                    <div className="space-y-2">
                                        {searchResults.map((user) => (
                                            <button key={user.id} onClick={() => startConversation(user)} className="w-full flex items-center gap-3 p-3 glass-panel hover:bg-glass-hover rounded-lg transition-colors border-transparent">
                                                {user.profile_image_url ? (
                                                    <img src={user.profile_image_url} alt={user.display_name} className="w-11 h-11 rounded-full object-cover border border-white/5" />
                                                ) : (
                                                    <div className="w-11 h-11 rounded-full bg-accent/10 border border-accent/20 flex items-center justify-center">
                                                        <MessageCircle size={18} className="text-accent" />
                                                    </div>
                                                )}
                                                <div className="text-left">
                                                    <span className="text-textPrimary text-sm font-medium block">{user.display_name}</span>
                                                    <span className="text-textMuted text-xs">@{user.login}</span>
                                                </div>
                                            </button>
                                        ))}
                                    </div>
                                ) : searchQuery && !isSearching ? (
                                    <div className="text-center text-textMuted py-8">No users found</div>
                                ) : null}
                            </div>
                        </div>
                    ) : activeConversation && currentConversation ? (
                        <>
                            {/* Conversation Header */}
                            <div className="flex items-center gap-3 px-5 py-4 border-b border-borderSubtle shrink-0">
                                <button onClick={() => setActiveConversation(null)} className="p-2 text-textSecondary hover:text-accent hover:bg-surface-hover rounded-lg transition-colors md:hidden">
                                    <ArrowLeft size={18} />
                                </button>
                                
                                <button 
                                    onClick={handleProfileClick}
                                    className="flex items-center gap-3 flex-1 text-left group hover:bg-surface-hover p-1.5 -ml-1.5 rounded-lg transition-colors"
                                >
                                    {currentConversation.profile_image_url ? (
                                        <img src={currentConversation.profile_image_url} alt={currentConversation.user_name} className="w-10 h-10 rounded-full object-cover border border-white/5 group-hover:border-accent/40 transition-colors" />
                                    ) : (
                                        <div className="w-10 h-10 rounded-full bg-accent/10 border border-accent/20 flex items-center justify-center group-hover:border-accent/40 transition-colors">
                                            <MessageCircle size={16} className="text-accent" />
                                        </div>
                                    )}
                                    <div className="flex-1">
                                        <span className="text-textPrimary font-semibold text-sm block group-hover:text-accent transition-colors">{currentConversation.user_name}</span>
                                        <span className="text-textMuted text-xs">@{currentConversation.user_login}</span>
                                    </div>
                                </button>

                                <button onClick={handleClose} className="p-2 text-textSecondary hover:text-error hover:bg-red-500/10 rounded-lg transition-colors hidden md:block shrink-0">
                                    <X size={18} />
                                </button>
                            </div>

                            {/* Messages */}
                            <div className="flex-1 overflow-y-auto min-h-0 p-5 space-y-4 scrollbar-thin">
                                {currentConversation.messages.length === 0 ? (
                                    <div className="flex flex-col items-center justify-center h-full text-textMuted">
                                        <MessageCircle size={40} className="mb-3 opacity-50 text-accent/50" />
                                        <span className="text-sm">No messages yet</span>
                                        <span className="text-xs mt-1">Start the conversation!</span>
                                    </div>
                                ) : (
                                    currentConversation.messages.map((msg, index) => (
                                        <motion.div key={`${msg.id}-${index}`} initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} className={`flex ${msg.is_sent ? 'justify-end' : 'justify-start'}`}>
                                            <div className={`max-w-[75%] px-4 py-3 glass-button-static text-textPrimary ${msg.is_sent ? 'bg-blue-500/15 !border-blue-500/30 rounded-2xl rounded-br-md' : 'rounded-2xl rounded-bl-md'}`}>
                                                <p className="text-sm break-words whitespace-pre-wrap">{parseTextWithLinks(msg.message)}</p>
                                                <span className={`text-[10px] mt-1.5 block ${msg.is_sent ? 'text-blue-400/70 font-medium' : 'text-textMuted font-medium'}`}>{formatTime(msg.timestamp)}</span>
                                            </div>
                                        </motion.div>
                                    ))
                                )}
                                <div ref={messagesEndRef} />
                            </div>

                            {/* Error */}
                            {error && (
                                <div className="px-5 py-2 bg-red-500/10 border-t border-red-500/20">
                                    <span className="text-red-400 text-xs">{error}</span>
                                </div>
                            )}

                            {/* Input Area */}
                            <div className="shrink-0 p-4 relative bg-surface-hover/20 backdrop-blur-md border-t border-borderSubtle">
                                {/* Emoji Picker */}
                                {showEmojiPicker && (
                                    <div ref={emojiPickerRef} className="absolute bottom-full left-4 right-4 mb-2 glass-panel shadow-xl overflow-hidden" style={{ height: '320px' }}>
                                        <div className="p-3 border-b border-borderSubtle">
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
                                                        key={category}
                                                        onClick={() => { setSelectedEmojiCategory(category); setEmojiSearchQuery(''); }}
                                                        className={`px-3 py-1.5 text-xs font-medium rounded-md whitespace-nowrap transition-colors ${selectedEmojiCategory === category ? 'bg-accent/20 text-accent border border-transparent shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-accent)_35%,transparent)]' : 'bg-transparent text-textSecondary hover:bg-surface-hover border border-transparent'}`}
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
                                                        onClick={() => insertEmoji(emoji)}
                                                        className="flex items-center justify-center p-2 hover:bg-surface-hover rounded-lg transition-colors"
                                                    >
                                                        <img src={getAppleEmojiUrl(emoji)} alt={emoji} className="w-6 h-6 object-contain" onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; (e.target as HTMLImageElement).insertAdjacentText('afterend', emoji); }} />
                                                    </button>
                                                    </Tooltip>
                                                ))}
                                            </div>
                                        </div>
                                    </div>
                                )}

                                <div className="flex items-end gap-2">
                                    <button onClick={() => setShowEmojiPicker(!showEmojiPicker)} className={`p-2.5 rounded-xl transition-colors flex-shrink-0 ${showEmojiPicker ? 'text-accent bg-accent/10 border border-transparent shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-accent)_35%,transparent)]' : 'text-textSecondary hover:text-accent hover:bg-surface-hover border border-transparent'}`}>
                                        <Smile size={20} />
                                    </button>
                                    {/* Wrapper so the spell-check underlay can sit
                                        over the textarea; the textarea keeps the
                                        `flex-1` sizing it had on its own. */}
                                    <div className="relative flex-1 flex">
                                    <textarea
                                        ref={inputRef}
                                        value={message}
                                        onChange={(e) => setMessage(e.target.value)}
                                        onKeyDown={handleKeyDown}
                                        onFocus={warmSpellcheck}
                                        spellCheck={false}
                                        data-spellcheck={spellcheckEnabled ? 'true' : undefined}
                                        placeholder="Type a message..."
                                        disabled={isSending}
                                        maxLength={500}
                                        rows={1}
                                        className="w-full glass-input rounded-xl px-4 py-3 text-sm transition-all duration-200 disabled:opacity-50 resize-none min-h-[44px] max-h-[120px] shadow-sm"
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
                                            className="rounded-xl px-4 py-3 text-sm"
                                        />
                                    )}
                                    </div>
                                    <button onClick={handleSend} disabled={!message.trim() || isSending} className="p-2.5 glass-button hover:text-accent disabled:opacity-40 disabled:hover:text-textPrimary disabled:cursor-not-allowed text-textPrimary rounded-xl transition-colors flex-shrink-0">
                                        {isSending ? <Loader2 size={20} className="animate-spin text-accent" /> : <Send size={20} className={`${message.trim() ? 'text-accent drop-shadow-[0_0_8px_rgba(var(--color-accent-rgb),0.4)]' : ''} transition-all`} />}
                                    </button>
                                </div>
                                <div className="flex justify-between mt-2 px-2">
                                    <span className="text-[10px] text-textMuted/70 font-medium">Press Enter to send, Shift+Enter for new line</span>
                                    <span className="text-[10px] text-textMuted/70 font-medium">{message.length}/500</span>
                                </div>
                            </div>
                        </>
                    ) : (
                        <div className="flex-1 flex flex-col min-h-0 items-center justify-center text-textMuted p-8">
                            <MessageCircle size={56} className="mb-4 opacity-30 text-accent" />
                            <span className="text-lg font-medium mb-1 text-textPrimary">Select a conversation</span>
                            <span className="text-sm mb-6 text-textMuted">or start a new one to begin whispering</span>
                            <button onClick={() => setShowNewConversation(true)} className="px-5 py-2.5 glass-button hover:text-accent font-medium text-textPrimary rounded-lg text-sm transition-colors shadow-lg">
                                New Conversation
                            </button>
                            <button onClick={handleClose} className="mt-6 p-2 text-textSecondary hover:text-error hover:bg-red-500/10 rounded-lg transition-colors hidden md:block">
                                <X size={22} />
                            </button>
                        </div>
                    )}
                </div>
            </motion.div>

            {/* Import Wizard */}
            <WhisperImportWizard
                isOpen={showImportWizard}
                onClose={() => setShowImportWizard(false)}
            />
        </motion.div>
    );
};

export default WhispersWidget;
