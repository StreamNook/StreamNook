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
import type { WhisperConversation, Whisper, UserInfo } from '../types';

import { Logger } from '../utils/logger';
interface WhisperFromBackend {
    from_user_id: string;
    from_user_login: string;
    from_user_name: string;
    to_user_id: string;
    to_user_login: string;
    to_user_name: string;
    whisper_id: string;
    text: string;
}

interface ExportedMessage {
    id: string;
    fromUserId?: string;
    fromUserLogin?: string;
    fromUserName?: string;
    content: string;
    sentAt: string;
    isSent?: boolean;
}

interface ExportedConversation {
    user: {
        id?: string;
        login: string;
        displayName: string;
        profileImageURL?: string;
    };
    messages: ExportedMessage[];
    lastMessageAt?: string;
}

interface WhispersWidgetProps {
    isOpen: boolean;
    onClose: () => void;
}

type SortOption = 'recent' | 'name' | 'unread';

const WHISPERS_LOCALSTORAGE_KEY = 'streamnook-whisper-conversations';
const WHISPERS_MIGRATED_KEY = 'streamnook-whispers-migrated-to-disk';

// Parse various date formats from imported whispers
const parseWhisperDate = (dateStr: string): number => {
    if (!dateStr) return Date.now();

    // Try standard Date parsing first
    let timestamp = new Date(dateStr).getTime();
    if (!isNaN(timestamp)) return timestamp;

    // Try parsing locale format like "5/31/2021, 8:30:39 PM PDT"
    // Remove timezone abbreviation and try again
    const withoutTz = dateStr.replace(/\s+[A-Z]{2,4}$/, '');
    timestamp = new Date(withoutTz).getTime();
    if (!isNaN(timestamp)) return timestamp;

    // Fallback: try to extract date components manually
    const match = dateStr.match(/(\d{1,2})\/(\d{1,2})\/(\d{4}),?\s*(\d{1,2}):(\d{2}):?(\d{2})?\s*(AM|PM)?/i);
    if (match) {
        const [, month, day, year, hours, minutes, seconds, ampm] = match;
        let h = parseInt(hours);
        if (ampm) {
            if (ampm.toUpperCase() === 'PM' && h < 12) h += 12;
            if (ampm.toUpperCase() === 'AM' && h === 12) h = 0;
        }
        return new Date(parseInt(year), parseInt(month) - 1, parseInt(day), h, parseInt(minutes), parseInt(seconds || '0')).getTime();
    }

    return Date.now();
};

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

// Save conversations to disk via Tauri backend (scoped to the owning account)
const saveConversationsToDisk = async (conversations: Map<string, WhisperConversation>, ownerId: string) => {
    try {
        const conversationsObj = mapToObject(conversations);
        await invoke('save_whisper_storage', { ownerId, conversations: conversationsObj });
        Logger.debug('[Whispers] Saved to disk:', conversations.size, 'conversations');
    } catch (error) {
        Logger.warn('[Whispers] Failed to save to disk:', error);
    }
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

const WhispersWidget = ({ isOpen, onClose }: WhispersWidgetProps) => {
    const [conversations, setConversations] = useState<Map<string, WhisperConversation>>(new Map());
    const [isLoading, setIsLoading] = useState(true);
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

    // Listen for auto-import whisper data event
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        let isMounted = true;

        const setupListener = async () => {
            // Listen for the actual data object (emitted directly from backend)
            const unlistenFn = await listen<{ version: number; exportedAt: string; myUserId: string | null; myUsername: string | null; conversations: ExportedConversation[] }>('whisper-data-ready', async (event) => {
                const data = event.payload;
                Logger.debug('[Whispers] Auto-import data received directly:', data.conversations?.length, 'conversations');

                try {
                    if (!data.version || !data.conversations || !Array.isArray(data.conversations)) {
                        throw new Error('Invalid data format.');
                    }

                    Logger.debug(`[Whispers] Auto-importing ${data.conversations.length} conversations...`);

                    const myUsername = currentUser?.login || currentUser?.username || data.myUsername || '';
                    const myUserId = currentUser?.user_id || data.myUserId || '';
                    const myDisplayName = currentUser?.display_name || currentUser?.username || data.myUsername || '';

                    const newConversations = new Map(conversations);

                    for (const conv of data.conversations) {
                        const userId = conv.user.id || conv.user.login.toLowerCase();
                        const existing = newConversations.get(userId);
                        const importedMessages: Whisper[] = conv.messages.map((msg: ExportedMessage) => {
                            const isSent = msg.isSent === true || (msg.fromUserName && myUsername && msg.fromUserName.toLowerCase() === myUsername.toLowerCase());
                            return {
                                id: msg.id,
                                from_user_id: msg.fromUserId || (isSent ? myUserId : userId),
                                from_user_login: msg.fromUserLogin || (isSent ? myUsername : conv.user.login),
                                from_user_name: msg.fromUserName || (isSent ? myDisplayName : conv.user.displayName),
                                to_user_id: isSent ? userId : myUserId,
                                to_user_login: isSent ? conv.user.login : myUsername,
                                to_user_name: isSent ? conv.user.displayName : myDisplayName,
                                message: msg.content,
                                timestamp: parseWhisperDate(msg.sentAt),
                                is_sent: Boolean(isSent),
                            };
                        });

                        if (existing) {
                            const existingIds = new Set(existing.messages.map(m => m.id));
                            const uniqueNewMessages = importedMessages.filter(m => !existingIds.has(m.id));
                            existing.messages = [...existing.messages, ...uniqueNewMessages].sort((a, b) => a.timestamp - b.timestamp);
                            if (!existing.profile_image_url && conv.user.profileImageURL) {
                                existing.profile_image_url = conv.user.profileImageURL;
                            }
                            if (existing.messages.length > 0) {
                                existing.last_message_timestamp = existing.messages[existing.messages.length - 1].timestamp;
                            }
                        } else {
                            newConversations.set(userId, {
                                user_id: userId,
                                user_login: conv.user.login,
                                user_name: conv.user.displayName,
                                profile_image_url: conv.user.profileImageURL || undefined,
                                messages: importedMessages.sort((a, b) => a.timestamp - b.timestamp),
                                last_message_timestamp: conv.lastMessageAt ? parseWhisperDate(conv.lastMessageAt) : Date.now(),
                                unread_count: 0,
                            });
                        }
                    }

                    setConversations(newConversations);
                    const totalMessages = data.conversations.reduce((sum: number, conv: ExportedConversation) => sum + conv.messages.length, 0);
                    Logger.debug(`[Whispers] Auto-imported ${totalMessages} messages from ${data.conversations.length} conversations`);
                    setImportProgress(`✓ Auto-imported ${totalMessages} messages`);

                    // Resolve user IDs and fetch profile pictures for all imported users
                    // This ensures conversations use numeric Twitch user IDs as keys (not usernames)
                    const usersToResolve = Array.from(newConversations.entries()).filter(([key, c]) => {
                        // Need to resolve if: key is not numeric OR missing profile image
                        const needsIdResolution = !/^\d+$/.test(key);
                        return needsIdResolution || !c.profile_image_url;
                    });

                    if (usersToResolve.length > 0) {
                        Logger.debug(`[Whispers] Resolving ${usersToResolve.length} user IDs and profile pictures...`);
                        setImportProgress(`Resolving ${usersToResolve.length} user IDs...`);

                        const resolvedConversations = new Map(newConversations);

                        for (const [mapKey, conv] of usersToResolve) {
                            try {
                                // Use search_whisper_user to look up by login (username)
                                const result = await invoke<[string, string, string, string | null] | null>('search_whisper_user', { username: conv.user_login });
                                if (result && result[0]) {
                                    const [realUserId, , , profileUrl] = result;
                                    Logger.debug(`[Whispers] Resolved ${conv.user_login}: ${mapKey} -> ${realUserId}`);

                                    // Update the conversation with resolved data
                                    conv.user_id = realUserId;
                                    if (profileUrl) {
                                        conv.profile_image_url = profileUrl;
                                    }

                                    // If the key was a username (not numeric), migrate to numeric ID
                                    if (!/^\d+$/.test(mapKey)) {
                                        resolvedConversations.delete(mapKey);
                                        resolvedConversations.set(realUserId, conv);
                                        Logger.debug(`[Whispers] Migrated key from "${mapKey}" to "${realUserId}"`);
                                    }
                                }
                            } catch (err) {
                                Logger.warn(`[Whispers] Failed to resolve user ${conv.user_login}:`, err);
                            }
                        }

                        // Update state with resolved conversations
                        setConversations(resolvedConversations);
                        Logger.debug('[Whispers] User ID resolution complete');
                    }

                    setImportProgress(`✓ Auto-imported ${totalMessages} messages`);
                    setTimeout(() => setImportProgress(''), 3000);
                } catch (err) {
                    Logger.error('[Whispers] Failed to process auto-import data:', err);
                }
            });

            if (isMounted) unlisten = unlistenFn;
            else unlistenFn();
        };

        setupListener();

        return () => {
            isMounted = false;
            if (unlisten) {
                unlisten();
            }
        };
    }, [currentUser, conversations]);

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
            setIsLoading(false);
            return;
        }
        let cancelled = false;
        const loadConversations = async () => {
            setIsLoading(true);
            try {
                const loaded = await migrateFromLocalStorage(ownerId);
                if (!cancelled) setConversations(loaded);
            } catch (error) {
                Logger.error('[Whispers] Failed to load conversations:', error);
            } finally {
                if (!cancelled) setIsLoading(false);
            }
        };
        loadConversations();
        return () => { cancelled = true; };
    }, [currentUser?.user_id]);

    // Save conversations to disk whenever they change (debounced), under the
    // signed-in account. Never writes while signed out.
    useEffect(() => {
        if (isLoading) return; // Don't save during initial load
        const ownerId = currentUser?.user_id;
        if (!ownerId) return;
        if (conversations.size > 0) {
            saveConversationsToDisk(conversations, ownerId);
        }
    }, [conversations, isLoading, currentUser?.user_id]);

    // Pull the latest messages for existing conversations straight from Twitch
    // (web session token + whisper GQL) and merge anything new into history by
    // timestamp — e.g. replies you sent from the Twitch site or your phone that
    // StreamNook never saw. Cheap: one recent page per conversation.
    const handleRefresh = async () => {
        const myUserId = currentUser?.user_id;
        if (!myUserId || isRefreshing) return;
        const userIds = Array.from(conversations.keys()).filter((id) => /^\d+$/.test(id));
        if (userIds.length === 0) {
            setImportProgress('No conversations to refresh');
            setTimeout(() => setImportProgress(''), 2500);
            return;
        }
        setIsRefreshing(true);
        setImportProgress('Refreshing messages…');
        try {
            const byUser = await invoke<Record<string, Array<{ id: string; from_user_id: string; from_user_name: string; content: string; sent_at: string }>>>(
                'refresh_whisper_history',
                { userIds }
            );
            const myLogin = currentUser?.login || currentUser?.username || '';
            const myName = currentUser?.display_name || currentUser?.username || '';
            const merged = new Map(conversations);
            let added = 0;
            for (const [otherId, msgs] of Object.entries(byUser)) {
                const conv = merged.get(otherId);
                if (!conv) continue;
                const existingIds = new Set(conv.messages.map((m) => m.id));
                const fresh: Whisper[] = [];
                for (const m of msgs) {
                    if (existingIds.has(m.id)) continue;
                    const isSent = m.from_user_id === myUserId;
                    fresh.push({
                        id: m.id,
                        from_user_id: m.from_user_id,
                        from_user_login: isSent ? myLogin : conv.user_login,
                        from_user_name: m.from_user_name || (isSent ? myName : conv.user_name),
                        to_user_id: isSent ? otherId : myUserId,
                        to_user_login: isSent ? conv.user_login : myLogin,
                        to_user_name: isSent ? conv.user_name : myName,
                        message: m.content,
                        timestamp: parseWhisperDate(m.sent_at),
                        is_sent: isSent,
                    });
                }
                if (fresh.length === 0) continue;
                const newMessages = [...conv.messages, ...fresh].sort((a, b) => a.timestamp - b.timestamp);
                merged.set(otherId, {
                    ...conv,
                    messages: newMessages,
                    last_message_timestamp: newMessages[newMessages.length - 1].timestamp,
                });
                added += fresh.length;
            }
            if (added > 0) setConversations(merged); // scoped save effect persists it
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

    // Play notification sound
    const playNotificationSound = useCallback(() => {
        try {
            const audioContext = new (window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext)();
            const oscillator = audioContext.createOscillator();
            const gainNode = audioContext.createGain();
            oscillator.connect(gainNode);
            gainNode.connect(audioContext.destination);
            oscillator.type = 'sine';
            oscillator.frequency.setValueAtTime(520, audioContext.currentTime);
            oscillator.frequency.exponentialRampToValueAtTime(380, audioContext.currentTime + 0.12);
            gainNode.gain.setValueAtTime(0, audioContext.currentTime);
            gainNode.gain.linearRampToValueAtTime(0.07, audioContext.currentTime + 0.02);
            gainNode.gain.exponentialRampToValueAtTime(0.001, audioContext.currentTime + 0.25);
            oscillator.start(audioContext.currentTime);
            oscillator.stop(audioContext.currentTime + 0.25);
        } catch (err) {
            Logger.warn('Could not play notification sound:', err);
        }
    }, []);

    // Listen for incoming whispers
    useEffect(() => {
        let isMounted = true;
        let unlistenFn: (() => void) | undefined;

        const setupListener = async () => {
            const unlisten = await listen<WhisperFromBackend>('whisper-received', async (event) => {
                const data = event.payload;
                if (settings.live_notifications?.play_sound) {
                playNotificationSound();
            }
            let profileImageUrl: string | undefined;
            try {
                const userInfo = await invoke<UserInfo>('get_user_by_id', { userId: data.from_user_id });
                profileImageUrl = userInfo.profile_image_url;
            } catch {
                // Ignore
            }
            const whisperMessage: Whisper = {
                id: data.whisper_id,
                from_user_id: data.from_user_id,
                from_user_login: data.from_user_login,
                from_user_name: data.from_user_name,
                to_user_id: data.to_user_id,
                to_user_login: data.to_user_login,
                to_user_name: data.to_user_name,
                message: data.text,
                timestamp: Date.now(),
                is_sent: false,
            };
            setConversations(prev => {
                const newConversations = new Map(prev);

                // First try to find by numeric user ID
                let existing = newConversations.get(data.from_user_id);
                let existingKey = data.from_user_id;

                // If not found, check if there's a conversation keyed by username (from imported data)
                // This handles the case where imported conversations use username as key
                if (!existing) {
                    const loginLower = data.from_user_login.toLowerCase();
                    // Check for conversation keyed by login
                    if (newConversations.has(loginLower)) {
                        existing = newConversations.get(loginLower);
                        existingKey = loginLower;
                    } else {
                        // Also check all conversations by user_login match
                        for (const [key, conv] of newConversations.entries()) {
                            if (conv.user_login.toLowerCase() === loginLower) {
                                existing = conv;
                                existingKey = key;
                                break;
                            }
                        }
                    }
                }

                if (existing) {
                    // Conversations live in React state: build a new object and a
                    // new message array rather than pushing into the old ones, so
                    // memoized consumers (and the compiler) see the change.
                    const messageExists = existing.messages.some(m => m.id === whisperMessage.id);
                    const updated: WhisperConversation = messageExists
                        ? { ...existing }
                        : {
                            ...existing,
                            messages: [...existing.messages, whisperMessage],
                            last_message_timestamp: Date.now(),
                            unread_count:
                                activeConversation !== existingKey && activeConversation !== data.from_user_id
                                    ? existing.unread_count + 1
                                    : existing.unread_count,
                        };

                    // If the conversation was found by username key, migrate it to use numeric ID
                    if (existingKey !== data.from_user_id) {
                        updated.user_id = data.from_user_id;
                        // Update profile image if we have it and they don't
                        if (profileImageUrl && !updated.profile_image_url) {
                            updated.profile_image_url = profileImageUrl;
                        }
                        // Remove old key and add with new numeric ID key
                        newConversations.delete(existingKey);
                        newConversations.set(data.from_user_id, updated);
                        Logger.debug(`[Whispers] Migrated conversation key from "${existingKey}" to "${data.from_user_id}"`);
                    } else {
                        newConversations.set(existingKey, updated);
                    }
                } else {
                    newConversations.set(data.from_user_id, {
                        user_id: data.from_user_id,
                        user_login: data.from_user_login,
                        user_name: data.from_user_name,
                        profile_image_url: profileImageUrl,
                        messages: [whisperMessage],
                        last_message_timestamp: Date.now(),
                        unread_count: activeConversation === data.from_user_id ? 0 : 1,
                    });
                }
                return newConversations;
            });
        });
        
            if (isMounted) unlistenFn = unlisten;
            else unlisten();
        };

        setupListener();

        return () => {
            isMounted = false;
            if (unlistenFn) unlistenFn();
        };
    }, [activeConversation, playNotificationSound, settings.live_notifications?.play_sound]);

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

        // Mark current conversation as read
        if (activeConversation) {
            setConversations(prev => {
                const newConversations = new Map(prev);
                const existing = newConversations.get(activeConversation);
                if (existing && existing.unread_count > 0) {
                    existing.unread_count = 0;
                }
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
        if (!file) return;
        setIsImportingAll(true);
        setImportProgress('Reading file...');
        try {
            const text = await file.text();
            const data = JSON.parse(text);
            if (!data.version || !data.conversations || !Array.isArray(data.conversations)) {
                throw new Error('Invalid file format.');
            }
            setImportProgress(`Importing ${data.conversations.length} conversations...`);
            const myUsername = currentUser?.login || currentUser?.username || data.myUsername || '';
            const myUserId = currentUser?.user_id || data.myUserId || '';
            const myDisplayName = currentUser?.display_name || currentUser?.username || data.myUsername || '';

            // First pass: import all conversations
            const newConversations = new Map(conversations);
            const usersNeedingProfilePics: string[] = [];

            for (const conv of data.conversations) {
                const userId = conv.user.id || conv.user.login.toLowerCase();
                const existing = newConversations.get(userId);
                const importedMessages: Whisper[] = conv.messages.map((msg: ExportedMessage) => {
                    const isSent = msg.isSent === true || (msg.fromUserName && myUsername && msg.fromUserName.toLowerCase() === myUsername.toLowerCase());
                    return {
                        id: msg.id,
                        from_user_id: msg.fromUserId || (isSent ? myUserId : userId),
                        from_user_login: msg.fromUserLogin || (isSent ? myUsername : conv.user.login),
                        from_user_name: msg.fromUserName || (isSent ? myDisplayName : conv.user.displayName),
                        to_user_id: isSent ? userId : myUserId,
                        to_user_login: isSent ? conv.user.login : myUsername,
                        to_user_name: isSent ? conv.user.displayName : myDisplayName,
                        message: msg.content,
                        timestamp: parseWhisperDate(msg.sentAt),
                        is_sent: Boolean(isSent),
                    };
                });
                if (existing) {
                    const existingIds = new Set(existing.messages.map(m => m.id));
                    const uniqueNewMessages = importedMessages.filter(m => !existingIds.has(m.id));
                    existing.messages = [...existing.messages, ...uniqueNewMessages].sort((a, b) => a.timestamp - b.timestamp);
                    if (!existing.profile_image_url && conv.user.profileImageURL) {
                        existing.profile_image_url = conv.user.profileImageURL;
                    }
                    if (!existing.profile_image_url) {
                        usersNeedingProfilePics.push(userId);
                    }
                    if (existing.messages.length > 0) {
                        existing.last_message_timestamp = existing.messages[existing.messages.length - 1].timestamp;
                    }
                } else {
                    newConversations.set(userId, {
                        user_id: userId,
                        user_login: conv.user.login,
                        user_name: conv.user.displayName,
                        profile_image_url: conv.user.profileImageURL || null,
                        messages: importedMessages.sort((a, b) => a.timestamp - b.timestamp),
                        last_message_timestamp: conv.lastMessageAt ? parseWhisperDate(conv.lastMessageAt) : Date.now(),
                        unread_count: 0,
                    });
                    if (!conv.user.profileImageURL) {
                        usersNeedingProfilePics.push(userId);
                    }
                }
            }

            setConversations(newConversations);
            const totalMessages = data.conversations.reduce((sum: number, conv: ExportedConversation) => sum + conv.messages.length, 0);
            setImportProgress(`✓ Imported ${totalMessages} messages`);

            // Second pass: Resolve user IDs and fetch profile pictures
            // This ensures all imported conversations have valid numeric Twitch user IDs
            setImportProgress(`Resolving ${newConversations.size} user IDs...`);
            const resolvedConversations = new Map<string, WhisperConversation>();

            for (const [originalKey, conv] of newConversations.entries()) {
                try {
                    // Check if user_id needs to be resolved (non-numeric)
                    const needsIdResolution = !conv.user_id || !/^\d+$/.test(conv.user_id);

                    if (needsIdResolution || !conv.profile_image_url) {
                        // Look up user by login to get numeric ID and profile picture
                        const result = await invoke<[string, string, string, string | null] | null>('search_whisper_user', { username: conv.user_login });
                        if (result && result[0]) {
                            const [realUserId, , , profileUrl] = result;
                            // Update the conversation with resolved data
                            conv.user_id = realUserId;
                            if (profileUrl) {
                                conv.profile_image_url = profileUrl;
                            }
                            // Store with the real user ID as key
                            resolvedConversations.set(realUserId, conv);
                            Logger.debug(`[Whispers] Resolved ${conv.user_login}: ${originalKey} -> ${realUserId}`);
                        } else {
                            // Couldn't resolve, keep original (might not be able to send to this user)
                            resolvedConversations.set(originalKey, conv);
                            Logger.warn(`[Whispers] Could not resolve user: ${conv.user_login}`);
                        }
                    } else {
                        // Already has valid user_id, just copy over
                        resolvedConversations.set(conv.user_id, conv);
                    }
                } catch (err) {
                    // Keep original on error
                    resolvedConversations.set(originalKey, conv);
                    Logger.warn(`[Whispers] Error resolving ${conv.user_login}:`, err);
                }
            }

            // Update state with resolved conversations
            setConversations(resolvedConversations);
            setImportProgress(`✓ Imported ${totalMessages} messages from ${resolvedConversations.size} users`);

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

    const generateMessageId = () => `sent-${Date.now()}-${Math.random().toString(36).substring(2, 11)}`;

    const handleSend = async () => {
        if (!message.trim() || isSending || !activeConversation || sendingRef.current) return;
        const conversation = conversations.get(activeConversation);
        if (!conversation) return;

        // Use the stored user_id from the conversation, not the map key
        // This handles imported conversations where the key might be the username
        let targetUserId = conversation.user_id;

        // If user_id is not numeric, try to look it up by username
        if (!targetUserId || !/^\d+$/.test(targetUserId)) {
            Logger.debug('[Whispers] User ID not numeric, looking up by login:', conversation.user_login);
            try {
                const result = await invoke<[string, string, string, string | null] | null>('search_whisper_user', { username: conversation.user_login });
                if (result && result[0]) {
                    targetUserId = result[0];
                    // Update the conversation with the correct user_id
                    setConversations(prev => {
                        const updated = new Map(prev);
                        const conv = updated.get(activeConversation);
                        if (conv) {
                            conv.user_id = targetUserId;
                            if (result[3]) {
                                conv.profile_image_url = result[3];
                            }
                        }
                        return updated;
                    });
                    Logger.debug('[Whispers] Found user ID:', targetUserId);
                } else {
                    setError('Cannot send: User not found on Twitch.');
                    return;
                }
            } catch (err) {
                Logger.error('[Whispers] Failed to look up user:', err);
                setError('Cannot send: Failed to look up user.');
                return;
            }
        }

        const messageToSend = message.trim();
        const messageId = generateMessageId();
        sendingRef.current = true;
        setMessage('');
        setIsSending(true);
        setError(null);
        setShowEmojiPicker(false);
        try {
            await invoke('send_whisper', { toUserId: targetUserId, message: messageToSend });
            const sentMessage: Whisper = {
                id: messageId,
                from_user_id: currentUser?.user_id || '',
                from_user_login: currentUser?.login || currentUser?.username || '',
                from_user_name: currentUser?.display_name || currentUser?.username || '',
                to_user_id: activeConversation,
                to_user_login: conversation.user_login,
                to_user_name: conversation.user_name,
                message: messageToSend,
                timestamp: Date.now(),
                is_sent: true,
            };
            setConversations(prev => {
                const n = new Map(prev);
                const existing = n.get(activeConversation);
                if (existing && !existing.messages.some(m => m.id === messageId)) {
                    // New object + array: state is never mutated in place.
                    n.set(activeConversation, {
                        ...existing,
                        messages: [...existing.messages, sentMessage],
                        last_message_timestamp: Date.now(),
                    });
                }
                return n;
            });
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
