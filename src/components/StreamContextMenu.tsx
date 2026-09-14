import React, { useEffect, useRef, useState } from 'react';
import { streamProvider } from '../utils/streamProvider';
import { favoriteIdOf, favoriteMetaOf } from '../utils/favorites';
import { useContextMenuStore } from '../stores/contextMenuStore';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { LayoutGrid, Heart, UserPlus, UserMinus, Loader2, Scissors, Copy, ClipboardPaste, Type, User, MessageSquarePlus, Share2, Check, SpellCheck, BookPlus } from 'lucide-react';
import { Logger } from '../utils/logger';
import { invoke } from '@tauri-apps/api/core';
import { buildShareUrl } from '../utils/shareLink';
import { replaceInputRange } from '../utils/chatInputWord';
import { addToCustomDictionary } from '../utils/spellcheck';
import { Tooltip } from './ui/Tooltip';
import { gridRefusal } from '../types/providers';

export const StreamContextMenu: React.FC = () => {
    const { isOpen, x, y, stream, inputElement, selectionText, menuType, spellStatus, spell, isFollowing, isCheckingFollow, closeMenu, toggleFollow } = useContextMenuStore();
    const { toggleFavoriteStreamer, isFavoriteStreamer, setProfileModalUser } = useAppStore();
    const { addSlot, slots } = usemultiNookStore();
    
    const menuRef = useRef<HTMLDivElement>(null);
    const [copied, setCopied] = useState(false);

    // Reset the "copied" confirmation whenever the menu closes or reopens.
    useEffect(() => { if (!isOpen) setCopied(false); }, [isOpen]);

    // Close on escape key
    useEffect(() => {
        if (!isOpen) return;
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') closeMenu();
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [isOpen, closeMenu]);

    // Positioning: pin the menu's pointer-side EDGE to the click, rather than
    // offsetting the top-left corner by a guessed height.
    //
    // The old approach set `top = y - ESTIMATED_HEIGHT` when flipping upward,
    // which is only correct if the guess matches the real height. The input menu
    // has three very different heights (4 rows when there's nothing to correct,
    // ~6 with one suggestion, ~10 with five plus "Add to dictionary"), and the
    // composer sits at the bottom of the window so it flips every time. Any gap
    // between the guess and reality showed up as the menu floating well above
    // the pointer. Anchoring `bottom` instead means the menu meets the pointer
    // at whatever height it happens to be, and grows away from it.
    const MENU_WIDTH = 200;
    const ESTIMATED_HEIGHT = menuType === 'input' ? 220 : 215;
    const EDGE_GAP = 8;

    const viewportWidth = typeof window === 'undefined' ? 0 : window.innerWidth;
    const viewportHeight = typeof window === 'undefined' ? 0 : window.innerHeight;

    // The estimate now only decides WHICH edge to pin, never where it lands, so
    // being off by a few rows costs nothing.
    const flipUp = y + ESTIMATED_HEIGHT > viewportHeight && y > viewportHeight - y;
    const flipLeft = x + MENU_WIDTH > viewportWidth;

    // Written out in full rather than assembled from fragments: Tailwind only
    // emits classes it can find as literal strings in the source.
    const transformOrigin = flipUp
        ? (flipLeft ? 'origin-bottom-right' : 'origin-bottom-left')
        : (flipLeft ? 'origin-top-right' : 'origin-top-left');

    const placement: React.CSSProperties = {
        ...(flipUp ? { bottom: viewportHeight - y } : { top: y }),
        ...(flipLeft ? { right: viewportWidth - x } : { left: x }),
        // A menu taller than the space it has scrolls rather than running off
        // the screen — reachable either way.
        maxHeight: Math.max(120, (flipUp ? y : viewportHeight - y) - EDGE_GAP),
    };

    if (!isOpen) return null;

    if (menuType === 'input' && inputElement) {
        const handleCut = (e: React.MouseEvent) => {
            e.stopPropagation();
            inputElement.focus();
            document.execCommand('cut');
            closeMenu();
        };

        const handleCopy = (e: React.MouseEvent) => {
            e.stopPropagation();
            inputElement.focus();
            document.execCommand('copy');
            closeMenu();
        };

        const handlePaste = async (e: React.MouseEvent) => {
            e.stopPropagation();
            try {
                // Bypass Tauri frontend capabilities and call Rust backend directly
                const text = await invoke<string>('read_clipboard_text_native');
                if (!text) {
                    closeMenu();
                    return;
                }

                const start =
                    inputElement instanceof HTMLInputElement || inputElement instanceof HTMLTextAreaElement
                        ? inputElement.selectionStart ?? 0
                        : 0;
                const end =
                    inputElement instanceof HTMLInputElement || inputElement instanceof HTMLTextAreaElement
                        ? inputElement.selectionEnd ?? 0
                        : 0;

                replaceInputRange(inputElement, start, end, text);
            } catch (err) {
                Logger.error("Failed to paste", err);
            }
            closeMenu();
        };

        const handleSelectAll = (e: React.MouseEvent) => {
            e.stopPropagation();
            inputElement.focus();
            document.execCommand('selectAll');
            closeMenu();
        };

        const applySuggestion = (e: React.MouseEvent, suggestion: string) => {
            e.stopPropagation();
            if (!spell) return;
            // Only the letters are replaced — the range excludes surrounding
            // punctuation, so "recieve," becomes "receive," and not "receive".
            replaceInputRange(inputElement, spell.start, spell.end, suggestion);
            closeMenu();
        };

        const handleAddToDictionary = (e: React.MouseEvent) => {
            e.stopPropagation();
            if (!spell) return;
            void addToCustomDictionary(spell.word);
            closeMenu();
        };

        const rowClass = 'flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-white hover:bg-glass-hover transition-all';

        return (
            <div 
                className="fixed inset-0 z-[100] cursor-default"
                onPointerDown={(e) => {
                    e.stopPropagation();
                    closeMenu();
                }}
                onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    closeMenu();
                }}
            >
                <div
                    ref={menuRef}
                    className={`absolute w-44 glass-panel rounded-xl flex flex-col p-1 shadow-2xl overflow-y-auto scrollbar-thin animate-in fade-in zoom-in-95 duration-150 ${transformOrigin}`}
                    style={placement}
                    onPointerDown={(e) => e.stopPropagation()}
                    onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                    }}
                >
                    {/* Spelling sits above the editing actions: when there IS a
                        correction to offer, it's the reason the menu was opened. */}
                    {spellStatus === 'checking' && (
                        <div className="flex items-center gap-2 px-3 py-2 text-sm font-medium text-textMuted">
                            <Loader2 size={16} className="animate-spin" />
                            <span>Checking spelling...</span>
                        </div>
                    )}
                    {spellStatus === 'ready' && spell && (
                        <>
                            {spell.suggestions.map((suggestion) => (
                                <button
                                    key={suggestion}
                                    onClick={(e) => applySuggestion(e, suggestion)}
                                    className={rowClass}
                                >
                                    <SpellCheck size={16} />
                                    <span className="truncate">{suggestion}</span>
                                </button>
                            ))}
                            <button onClick={handleAddToDictionary} className={rowClass}>
                                <BookPlus size={16} />
                                <span>Add to dictionary</span>
                            </button>
                            <div className="h-px bg-borderSubtle my-1 mx-2" />
                        </>
                    )}
                    <button onClick={handleCut} className={rowClass}>
                        <Scissors size={16} />
                        <span>Cut</span>
                    </button>
                    <button onClick={handleCopy} className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-white hover:bg-glass-hover transition-all">
                        <Copy size={16} />
                        <span>Copy</span>
                    </button>
                    <button onClick={handlePaste} className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-white hover:bg-glass-hover transition-all">
                        <ClipboardPaste size={16} />
                        <span>Paste</span>
                    </button>
                    <div className="h-px bg-borderSubtle my-1 mx-2" />
                    <button onClick={handleSelectAll} className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-white hover:bg-glass-hover transition-all">
                        <Type size={16} />
                        <span>Select All</span>
                    </button>
                </div>
            </div>
        );
    }

    if (menuType === 'selection') {
        const handleCopy = async (e: React.MouseEvent) => {
            e.stopPropagation();
            if (selectionText) {
                try {
                    await navigator.clipboard.writeText(selectionText);
                } catch (err) {
                    Logger.error("Failed to copy", err);
                }
            }
            closeMenu();
        };

        return (
            <div 
                className="fixed inset-0 z-[100] cursor-default"
                onPointerDown={(e) => {
                    e.stopPropagation();
                    closeMenu();
                }}
                onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    closeMenu();
                }}
            >
                <div
                    ref={menuRef}
                    className={`absolute w-44 glass-panel rounded-xl flex flex-col p-1 shadow-2xl overflow-y-auto scrollbar-thin animate-in fade-in zoom-in-95 duration-150 ${transformOrigin}`}
                    style={placement}
                    onPointerDown={(e) => e.stopPropagation()}
                    onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                    }}
                >
                    <button onClick={handleCopy} className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-white hover:bg-glass-hover transition-all">
                        <Copy size={16} />
                        <span>Copy</span>
                    </button>
                </div>
            </div>
        );
    }

    if (!stream || menuType !== 'stream') return null;

    // `favoriteIdOf`, never the raw `user_id`: Kick and Twitch both use numeric
    // ids, so right-clicking a Kick row used to write a key that could collide
    // with a Twitch favourite (and never resolve as live either way).
    const favoriteId = favoriteIdOf(stream);
    const isFavorite = !!favoriteId && isFavoriteStreamer(favoriteId);
    const hasRoomForMultiNook = slots.length < 25;
    // Two separate reasons the item can be unavailable, and they read
    // differently: the grid is full, or this platform cannot be a tile at all.
    const gridProvider = streamProvider(stream);
    const providerRefusal = gridRefusal(gridProvider);
    const canAddToGrid = hasRoomForMultiNook && !providerRefusal;

    const handleFavorite = (e: React.MouseEvent) => {
        e.stopPropagation();
        if (!favoriteId) return;
        // Identity rides along so the channel can still be drawn once offline.
        void toggleFavoriteStreamer(favoriteId, favoriteMetaOf(stream, favoriteId));
        closeMenu();
    };

    const handleMultiNook = (e: React.MouseEvent) => {
        e.stopPropagation();
        if (!canAddToGrid) return;

        // Trigger flying animation from context menu click position
        usemultiNookStore.getState().triggerAddAnimation(x, y, stream.user_login, gridProvider);
        addSlot(stream.user_login, gridProvider);

        closeMenu();
    };

    // Open the streamer's chat in a new MultiChat popout window without
    // starting their stream. Useful when you want chat-only mode without
    // committing CPU/bandwidth to playing the video — the popout polls live
    // metadata (viewer count, uptime, etc.) but never starts playback.
    const handleOpenInMultiChat = async (e: React.MouseEvent) => {
        e.stopPropagation();
        closeMenu();
        try {
            const { openMultiChatWindow } = await import('../utils/multichatWindow');
            await openMultiChatWindow({
                channel: stream.user_login,
                channelId: stream.user_id || undefined,
                channelName: stream.user_name || undefined,
            });
        } catch (err) {
            Logger.error('[StreamContextMenu] openMultiChatWindow failed:', err);
        }
    };

    // Copy the shareable web link. It resolves on streamnook.app, which opens the
    // app when installed and falls back to Twitch otherwise. Shows an inline check
    // confirmation, then dismisses so the menu isn't left hanging.
    const handleShare = async (e: React.MouseEvent) => {
        e.stopPropagation();
        try {
            await navigator.clipboard.writeText(buildShareUrl(stream.user_login));
            setCopied(true);
            window.setTimeout(() => { setCopied(false); closeMenu(); }, 1100);
        } catch (err) {
            Logger.error('[StreamContextMenu] Failed to copy share link:', err);
            closeMenu();
        }
    };

    const handleFollow = (e: React.MouseEvent) => {
        e.stopPropagation();
        if (isCheckingFollow) return;
        toggleFollow();
    };

    return (
        // Backdrop overlay to capture clicks outside
        <div 
            className="fixed inset-0 z-[100] cursor-default"
            onPointerDown={(e) => {
                e.stopPropagation();
                closeMenu();
            }}
            onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                closeMenu();
            }}
        >
            {/* Global SVG Definitions for Liquid Glass Heart (Local Fallback) */}
            <svg width="0" height="0" className="absolute pointer-events-none">
                <defs>
                    <linearGradient id="glass-heart-fill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="rgba(255, 255, 255, 0.4)" />
                        <stop offset="30%" stopColor="rgba(236, 72, 153, 0.2)" />
                        <stop offset="100%" stopColor="rgba(236, 72, 153, 0.6)" />
                    </linearGradient>
                    <linearGradient id="glass-heart-stroke" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="rgba(255, 255, 255, 0.8)" />
                        <stop offset="100%" stopColor="rgba(255, 255, 255, 0.1)" />
                    </linearGradient>
                </defs>
            </svg>

            <div
                ref={menuRef}
                className={`absolute w-48 glass-panel rounded-xl flex flex-col p-1 shadow-2xl overflow-y-auto scrollbar-thin animate-in fade-in zoom-in-95 duration-150 ${transformOrigin}`}
                style={placement}
                onPointerDown={(e) => e.stopPropagation()} // Prevent closing when interacting with menu
                onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                }}
            >
                {/* Header (View Profile Action) */}
                <button 
                    onClick={(e) => {
                        e.stopPropagation();
                        setProfileModalUser(stream);
                        closeMenu();
                    }}
                    className="w-full text-left px-3 py-2 border-b border-borderSubtle mb-1 rounded-t-lg hover:bg-glass-hover transition-colors group flex items-center justify-between"
                >
                    <div>
                        <span className="text-xs font-semibold text-textPrimary group-hover:text-accent transition-colors truncate block">
                            {stream.user_name}
                        </span>
                        <span className="text-[10px] text-textMuted uppercase tracking-wider block mt-0.5 group-hover:text-accent/70 transition-colors">
                            View Profile
                        </span>
                    </div>
                    <User size={14} className="text-textMuted group-hover:text-accent transition-colors" />
                </button>

                {/* Add to MultiNook */}
                <Tooltip
                    content={providerRefusal ?? (hasRoomForMultiNook ? '' : 'The grid is full')}
                    side="right"
                    disabled={canAddToGrid}
                >
                <button
                    onClick={handleMultiNook}
                    disabled={!canAddToGrid}
                    className={`flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium transition-all w-full ${
                        canAddToGrid
                            ? 'text-textSecondary hover:text-white hover:bg-glass-hover'
                            : 'text-textMuted opacity-50 cursor-not-allowed'
                    }`}
                >
                    <LayoutGrid size={16} />
                    <span>Add to MultiNook</span>
                </button>
                </Tooltip>

                {/* Open chat in MultiChat — spawns a popout window pre-loaded
                    with this channel. Doesn't start the stream; chat only. */}
                <button
                    onClick={handleOpenInMultiChat}
                    className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-accent hover:bg-glass-hover transition-all"
                >
                    <MessageSquarePlus size={16} />
                    <span>Pop out chat</span>
                </button>

                {/* Share. Copies a streamnook.app/w/<channel> link. The icon and
                    label swap to a check + confirmation on copy, with a little pop. */}
                <button
                    onClick={handleShare}
                    className={`flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium transition-all ${
                        copied
                            ? 'text-green-400'
                            : 'text-textSecondary hover:text-accent hover:bg-glass-hover'
                    }`}
                >
                    <span key={copied ? 'copied' : 'share'} className="inline-flex animate-in zoom-in-50 duration-200">
                        {copied ? <Check size={16} /> : <Share2 size={16} />}
                    </span>
                    <span>{copied ? 'Link copied!' : 'Share'}</span>
                </button>

                {/* Favorite */}
                {favoriteId && (
                <button
                    onClick={handleFavorite}
                    className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-pink-400 hover:bg-glass-hover transition-all"
                >
                    <Heart 
                        size={16} 
                        fill={isFavorite ? 'url(#glass-heart-fill)' : 'none'} 
                        stroke={isFavorite ? 'url(#glass-heart-stroke)' : 'currentColor'}
                        strokeWidth={isFavorite ? 1.5 : 2}
                        className={isFavorite ? 'drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]' : ''} 
                    />
                    <span>{isFavorite ? 'Unfavorite' : 'Favorite'}</span>
                </button>
                )}

                {/* Follow / Unfollow */}
                <button
                    onClick={handleFollow}
                    disabled={isCheckingFollow || isFollowing === null}
                    className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-purple-400 hover:bg-glass-hover transition-all"
                >
                    {isCheckingFollow ? (
                        <>
                            <Loader2 size={16} className="animate-spin text-textMuted" />
                            <span className="text-textMuted">Checking...</span>
                        </>
                    ) : isFollowing ? (
                        <>
                            <UserMinus size={16} className="text-red-400" />
                            <span className="text-red-400">Unfollow</span>
                        </>
                    ) : (
                        <>
                            <UserPlus size={16} className="text-green-400" />
                            <span className="text-green-400">Follow</span>
                        </>
                    )}
                </button>
            </div>
        </div>
    );
};

