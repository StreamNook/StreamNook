import React, { useEffect, useRef, useState } from 'react';
import {
  Maximize2,
  Minimize2,
  Undo2,
  X as XIcon,
  MessageSquarePlus,
  Share2,
  Check,
  User,
  UserPlus,
  UserMinus,
  Loader2,
  Radio,
} from 'lucide-react';
import { useContextMenuStore } from '../../stores/contextMenuStore';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useAppStore } from '../../stores/AppStore';
import { buildShareUrl } from '../../utils/shareLink';
import { Logger } from '../../utils/logger';

const ROW =
  'flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-textSecondary transition-all hover:bg-glass-hover hover:text-white';

/**
 * Right-click menu for one MultiNook tile.
 *
 * A sibling of `StreamContextMenu` rather than another branch inside it: that
 * component answers for a channel ROW anywhere in the app, while this one acts
 * on a tile that is currently playing, and the two share almost no actions. They
 * share the store instead, so only one menu can ever be open and the follow
 * lookup is resolved the same way for both.
 *
 * The headline action is "Watch only this one", which closes the rest of the
 * grid and leaves this stream playing in the ordinary player. See
 * `promoteSlotToSolo`: the picture does not stop, because the relay already
 * serving this tile is handed straight to the solo player.
 */
export const MultiNookTileMenu: React.FC = () => {
  const {
    isOpen,
    x,
    y,
    stream,
    slotId,
    menuType,
    isFollowing,
    isCheckingFollow,
    closeMenu,
    toggleFollow,
  } = useContextMenuStore();

  const slots = usemultiNookStore((s) => s.slots);
  const maximizedSlotId = usemultiNookStore((s) => s.maximizedSlotId);
  const [copied, setCopied] = useState(false);

  // Every way out of the menu clears the share confirmation, so it is never
  // left standing for the next time the menu opens. Deliberately NOT an effect
  // keyed on `isOpen`: that is a setState cascade for something all three exits
  // already know about, and the repo's lint says so.
  //
  // Plain function rather than a useCallback: the escape listener below does
  // the same two calls inline so it can keep `closeMenu` (store-stable) as its
  // only dependency, which leaves nothing here that needs a stable identity.
  const dismiss = () => {
    setCopied(false);
    closeMenu();
  };

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      setCopied(false);
      closeMenu();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen, closeMenu]);

  // Same anchoring rule as StreamContextMenu: pin the pointer-side EDGE to the
  // click rather than offsetting a corner by a guessed height, so the menu meets
  // the pointer whatever height it ends up being. A tile menu is opened near the
  // bottom of the grid often enough for this to matter on every other click.
  const MENU_WIDTH = 210;
  const ESTIMATED_HEIGHT = 300;
  const EDGE_GAP = 8;
  const viewportWidth = typeof window === 'undefined' ? 0 : window.innerWidth;
  const viewportHeight = typeof window === 'undefined' ? 0 : window.innerHeight;
  const flipUp = y + ESTIMATED_HEIGHT > viewportHeight && y > viewportHeight - y;
  const flipLeft = x + MENU_WIDTH > viewportWidth;
  const transformOrigin = flipUp
    ? flipLeft
      ? 'origin-bottom-right'
      : 'origin-bottom-left'
    : flipLeft
      ? 'origin-top-right'
      : 'origin-top-left';

  const menuRef = useRef<HTMLDivElement>(null);

  if (!isOpen || menuType !== 'multinook-tile' || !stream || !slotId) return null;

  const slot = slots.find((s) => s.id === slotId);
  if (!slot) return null;

  const {
    promoteSlotToSolo,
    toggleMaximizeSlot,
    dockSlot,
    undockSlot,
    removeSlot,
    toggleFocusSlot,
  } = usemultiNookStore.getState();

  const isMaximized = maximizedSlotId === slotId;
  const isDocked = !!slot.isMinimized;
  const label = slot.channelName || slot.channelLogin;
  // "Close the others" only means something when there ARE others.
  const hasOthers = slots.length > 1;

  const placement: React.CSSProperties = {
    ...(flipUp ? { bottom: viewportHeight - y } : { top: y }),
    ...(flipLeft ? { right: viewportWidth - x } : { left: x }),
    maxHeight: Math.max(120, (flipUp ? y : viewportHeight - y) - EDGE_GAP),
  };

  const run = (fn: () => void) => (e: React.MouseEvent) => {
    e.stopPropagation();
    fn();
    dismiss();
  };

  const handlePromote = (e: React.MouseEvent) => {
    e.stopPropagation();
    dismiss();
    void promoteSlotToSolo(slotId);
  };

  const handlePopOutChat = async (e: React.MouseEvent) => {
    e.stopPropagation();
    dismiss();
    try {
      const { openMultiChatWindow } = await import('../../utils/multichatWindow');
      await openMultiChatWindow({
        channel: slot.channelLogin,
        channelId: slot.channelId || undefined,
        channelName: slot.channelName || undefined,
      });
    } catch (err) {
      Logger.error('[MultiNookTileMenu] openMultiChatWindow failed:', err);
    }
  };

  const handleShare = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(buildShareUrl(slot.channelLogin));
      setCopied(true);
      window.setTimeout(dismiss, 1100);
    } catch (err) {
      Logger.error('[MultiNookTileMenu] Failed to copy share link:', err);
      dismiss();
    }
  };

  return (
    <div
      className="fixed inset-0 z-[100] cursor-default"
      onPointerDown={(e) => {
        e.stopPropagation();
        dismiss();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        dismiss();
      }}
    >
      <div
        ref={menuRef}
        className={`absolute w-[210px] glass-panel rounded-xl flex flex-col p-1 shadow-2xl overflow-y-auto scrollbar-thin animate-in fade-in zoom-in-95 duration-150 ${transformOrigin}`}
        style={placement}
        onPointerDown={(e) => e.stopPropagation()}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
      >
        {/* Which tile this is acting on, and the way to the channel's profile. */}
        <button
          onClick={run(() => useAppStore.getState().setProfileModalUser(stream))}
          className="group mb-1 flex w-full items-center justify-between rounded-t-lg border-b border-borderSubtle px-3 py-2 text-left transition-colors hover:bg-glass-hover"
        >
          <div className="min-w-0">
            <span className="block truncate text-xs font-semibold text-textPrimary transition-colors group-hover:text-accent">
              {label}
            </span>
            <span className="mt-0.5 block text-[10px] uppercase tracking-wider text-textMuted transition-colors group-hover:text-accent/70">
              View profile
            </span>
          </div>
          <User size={14} className="shrink-0 text-textMuted transition-colors group-hover:text-accent" />
        </button>

        {/* The headline action. Worded as an outcome rather than as the two
            mechanical steps it performs, and it says "keeps playing" because
            that is the part people do not expect. */}
        <button
          onClick={handlePromote}
          className={`${ROW} !text-accent hover:!bg-accent/10`}
        >
          <Minimize2 size={16} />
          <span className="min-w-0 flex-1 text-left leading-tight">
            {hasOthers ? 'Watch only this one' : 'Leave the grid, keep watching'}
            <span className="mt-0.5 block text-[10px] font-normal normal-case text-textMuted">
              {hasOthers ? 'Closes the others, keeps playing' : 'Keeps playing, no reload'}
            </span>
          </span>
        </button>

        <div className="mx-2 my-1 h-px bg-borderSubtle" />

        {/* Tile actions, mirroring the hover overlay so neither surface has
            something the other lacks. Spotlight and dock are meaningless on a
            docked tile, which is offscreen. */}
        {!isDocked && (
          <>
            <button onClick={run(() => toggleMaximizeSlot(slotId))} className={ROW}>
              {isMaximized ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
              <span>{isMaximized ? 'Back to grid' : 'Spotlight'}</span>
            </button>

            {!slot.isFocused && (
              <button onClick={run(() => toggleFocusSlot(slotId))} className={ROW}>
                <Radio size={16} />
                <span>Focus audio and chat</span>
              </button>
            )}

            <button onClick={run(() => dockSlot(slotId))} className={ROW}>
              <Undo2 size={16} />
              <span>Dock</span>
            </button>
          </>
        )}

        {isDocked && (
          <button onClick={run(() => undockSlot(slotId))} className={ROW}>
            <Maximize2 size={16} />
            <span>Restore to grid</span>
          </button>
        )}

        <div className="mx-2 my-1 h-px bg-borderSubtle" />

        <button onClick={handlePopOutChat} className={`${ROW} hover:!text-accent`}>
          <MessageSquarePlus size={16} />
          <span>Pop out chat</span>
        </button>

        <button
          onClick={handleShare}
          className={`${ROW} ${copied ? '!text-green-400' : 'hover:!text-accent'}`}
        >
          <span key={copied ? 'copied' : 'share'} className="inline-flex animate-in zoom-in-50 duration-200">
            {copied ? <Check size={16} /> : <Share2 size={16} />}
          </span>
          <span>{copied ? 'Link copied' : 'Share'}</span>
        </button>

        <button
          onClick={(e) => {
            e.stopPropagation();
            if (isCheckingFollow) return;
            void toggleFollow();
          }}
          disabled={isCheckingFollow || isFollowing === null}
          className={`${ROW} hover:!text-purple-400`}
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

        <div className="mx-2 my-1 h-px bg-borderSubtle" />

        <button
          onClick={run(() => void removeSlot(slotId))}
          className={`${ROW} hover:!bg-error/10 hover:!text-error`}
        >
          <XIcon size={16} />
          <span>Close stream</span>
        </button>
      </div>
    </div>
  );
};

export default MultiNookTileMenu;
