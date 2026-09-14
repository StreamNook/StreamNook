import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';
import { useAppStore } from './AppStore';
import { Logger } from '../utils/logger';
import { getSpellcheckTarget, type SpellToken } from '../utils/chatInputWord';
import { isKnownChatToken, suggestWord } from '../utils/spellcheck';

/** The misspelled word the input menu is offering corrections for. */
export interface SpellSelection extends SpellToken {
    suggestions: string[];
}

interface ContextMenuState {
    isOpen: boolean;
    x: number;
    y: number;
    stream: TwitchStream | null;
    inputElement: HTMLElement | null;
    selectionText: string | null;
    menuType: 'stream' | 'input' | 'selection' | null;
    /** 'checking' while suggestions are being fetched; 'idle' when the clicked
     *  word is spelled fine, isn't checkable, or checking is off. */
    spellStatus: 'idle' | 'checking' | 'ready';
    spell: SpellSelection | null;
    isFollowing: boolean | null; // null means loading/unknown
    isCheckingFollow: boolean;
    
    // Actions
    openMenu: (e: React.MouseEvent | MouseEvent, stream: TwitchStream) => void;
    openInputMenu: (e: React.MouseEvent | MouseEvent, element: HTMLElement) => void;
    openSelectionMenu: (e: React.MouseEvent | MouseEvent) => void;
    closeMenu: () => void;
    toggleFollow: () => Promise<void>;
}

// Bumped every time the menu opens or closes. Suggestions arrive asynchronously,
// so a reply for a menu the user has already dismissed (or replaced by
// right-clicking somewhere else) has to be dropped rather than rendered.
let openGeneration = 0;

/**
 * Character offset in `element` under the given viewport point.
 *
 * `caretPositionFromPoint` hit-tests straight into a textarea's text and hands
 * back a real offset into its value, so the word we correct is the word the
 * pointer was actually over. Falls back to the caret when the API is missing or
 * the point landed on something else — the browser does move the caret on
 * right-click, that's just a weaker signal than the coordinates themselves.
 *
 * (Its older sibling `caretRangeFromPoint` is no use here: over a textarea it
 * returns a range in the wrapper element, not a character offset.)
 */
function offsetAtPoint(
    element: HTMLInputElement | HTMLTextAreaElement,
    x: number,
    y: number,
): number {
    const fromPoint = (
        document as Document & {
            caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
        }
    ).caretPositionFromPoint?.(x, y);

    if (fromPoint && fromPoint.offsetNode === element) return fromPoint.offset;
    return element.selectionStart ?? 0;
}

/** How long the menu waits for suggestions before opening without them.
 *  The engine is warmed when the composer takes focus, so this normally
 *  resolves in single-digit milliseconds and the menu opens once, complete,
 *  instead of opening short and growing under the pointer. */
const SUGGESTION_OPEN_DEADLINE_MS = 80;

/**
 * The word that was right-clicked, if it is one worth spell checking.
 *
 * Synchronous on purpose: whether there is anything to look up is decided
 * before the menu opens, so a right-click on an emote, a mention or a
 * non-composer input never pays for a round trip.
 */
function spellTargetFor(element: HTMLElement, x: number, y: number): SpellToken | null {
    if (element.dataset.spellcheck !== 'true') return null;
    if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return null;

    const caret = offsetAtPoint(element, x, y);

    // A selection the click landed inside is what the user means; anything else
    // and the pointer wins.
    const selStart = element.selectionStart ?? 0;
    const selEnd = element.selectionEnd ?? 0;
    const insideSelection = selEnd > selStart && caret >= selStart && caret <= selEnd;

    const target = insideSelection
        ? getSpellcheckTarget(element.value, selStart, selEnd)
        : getSpellcheckTarget(element.value, caret, caret);
    if (!target) return null;

    const emoteKey = element.dataset.spellcheckEmotes ?? null;
    return isKnownChatToken(target.word, { emoteKey }) ? null : target;
}

export const useContextMenuStore = create<ContextMenuState>((set, get) => ({
    isOpen: false,
    x: 0,
    y: 0,
    stream: null,
    inputElement: null,
    selectionText: null,
    menuType: null,
    spellStatus: 'idle',
    spell: null,
    isFollowing: null,
    isCheckingFollow: false,

    openMenu: async (e: React.MouseEvent | MouseEvent, stream: TwitchStream) => {
        // Prevent default window context menu and bubbling
        e.preventDefault();
        e.stopPropagation();
        openGeneration++;

        const x = e.clientX;
        const y = e.clientY;

        // Immediately open the menu with the stream context
        set({ 
            isOpen: true, 
            x, 
            y, 
            menuType: 'stream',
            stream, 
            inputElement: null,
            selectionText: null,
            spellStatus: 'idle',
            spell: null,
            isFollowing: null, 
            isCheckingFollow: true 
        });

        // Optimization: If the stream is already in our followedStreams cache,
        // we instantly know they are followed. No need to hit the API payload.
        const appStore = useAppStore.getState();
        const isAlreadyFollowed = appStore.followedStreams.some(s => s.user_id === stream.user_id);

        if (isAlreadyFollowed) {
            set({ isFollowing: true, isCheckingFollow: false });
            return;
        }

        // Otherwise, verify against the API (e.g. for streams from Discover or Search)
        try {
            const isFollowingApi = await invoke<boolean>('check_following_status', { targetUserId: stream.user_id });
            set({ isFollowing: isFollowingApi, isCheckingFollow: false });
        } catch (error) {
            Logger.warn('[ContextMenu] Failed to check follow status:', error);
            // Default to null / false on error
            set({ isFollowing: false, isCheckingFollow: false });
        }
    },

    openInputMenu: (e: React.MouseEvent | MouseEvent, element: HTMLElement) => {
        e.preventDefault();
        e.stopPropagation();

        const x = e.clientX;
        const y = e.clientY;

        element.focus();
        openGeneration++;
        const generation = openGeneration;

        const open = (spell: SpellSelection | null, spellStatus: ContextMenuState['spellStatus']) => {
            // Dropped if the user has already dismissed this menu or opened
            // another one somewhere else.
            if (generation !== openGeneration) return;
            set({
                isOpen: true,
                x,
                y,
                menuType: 'input',
                stream: null,
                inputElement: element,
                selectionText: null,
                spellStatus,
                spell,
                isFollowing: null,
                isCheckingFollow: false
            });
        };

        const target = spellTargetFor(element, x, y);
        if (!target) {
            open(null, 'idle');
            return;
        }

        const openWithoutSuggestions = setTimeout(
            () => open(null, 'checking'),
            SUGGESTION_OPEN_DEADLINE_MS,
        );

        void suggestWord(target.word)
            .then((verdict) => {
                clearTimeout(openWithoutSuggestions);
                // Spelled fine: no corrections, and no "Add to dictionary" for a
                // word the dictionary already knows.
                if (verdict.correct) {
                    open(null, 'idle');
                    return;
                }
                open({ ...target, suggestions: verdict.suggestions }, 'ready');
            })
            .catch((err) => {
                clearTimeout(openWithoutSuggestions);
                Logger.warn('[ContextMenu] spelling lookup failed:', err);
                open(null, 'idle');
            });
    },

    openSelectionMenu: (e: React.MouseEvent | MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        openGeneration++;

        const x = e.clientX;
        const y = e.clientY;
        const selectionText = window.getSelection()?.toString() || '';

        set({
            isOpen: true,
            x,
            y,
            menuType: 'selection',
            stream: null,
            inputElement: null,
            selectionText: selectionText || null,
            spellStatus: 'idle',
            spell: null,
            isFollowing: null,
            isCheckingFollow: false
        });
    },

    closeMenu: () => {
        openGeneration++;
        set({ isOpen: false, stream: null, inputElement: null, selectionText: null, menuType: null, spellStatus: 'idle', spell: null });
    },

    toggleFollow: async () => {
        const { stream, isFollowing } = get();
        if (!stream || isFollowing === null) return;

        // Optimistic UI update
        const newFollowingState = !isFollowing;
        set({ isFollowing: newFollowingState });

        try {
            if (newFollowingState) {
                await invoke('follow_channel', { targetUserId: stream.user_id });
                Logger.info(`[ContextMenu] Followed channel: ${stream.user_name}`);
            } else {
                await invoke('unfollow_channel', { targetUserId: stream.user_id });
                Logger.info(`[ContextMenu] Unfollowed channel: ${stream.user_name}`);
            }
            
            // Refresh the followed list if needed
            useAppStore.getState().loadFollowedStreams();
            
        } catch (error) {
            Logger.error('[ContextMenu] Failed to toggle follow:', error);
            // Revert optimistic update on failure
            set({ isFollowing: !newFollowingState });
        }
        
        // Close menu after action
        get().closeMenu();
    }
}));
