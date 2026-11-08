// Compact View and chat. By default Compact View is the picture alone; with
// "Keep chat" on (`keep_chat_in_compact`) chat stays where it is docked and the
// preset's window makes room for it, the way the aspect lock fits chat in any
// other window (Rust's `calculate_aspect_ratio_size`).

export type ChatPlacement = 'right' | 'left' | 'bottom' | 'hidden';

/** The resize divider between the player and docked chat, in CSS pixels. */
export const CHAT_SEPARATOR = 4;

/** Where chat sits while Compact View is on: hidden, unless the viewer keeps it. */
export function compactChatPlacement(placement: string, keepChat: boolean): string {
  return keepChat ? placement : 'hidden';
}

/** What docked chat takes out of a Compact View window: the arguments the
 *  aspect sizing command reads for it, or `null` when chat takes no room. */
export interface CompactChatSlot {
  placement: Exclude<ChatPlacement, 'hidden'>;
  chatSize: number;
  uiWidthOffset: number;
  uiHeightOffset: number;
}

export function compactChatSlot(input: {
  /** The placement the window is using now (already 'hidden' when chat is not kept). */
  placement: string;
  /** The docked chat's width (side) or height (bottom). */
  chatSize: number;
  /** Chat reveals on hover over the picture, so it takes no docked room. */
  revealOnHover: boolean;
  /** A MultiChat popout owns this channel's chat: the panel is gone. */
  chatInPopout: boolean;
}): CompactChatSlot | null {
  const { placement, chatSize, revealOnHover, chatInPopout } = input;
  if (chatInPopout || chatSize <= 0) return null;
  if (placement === 'right' || placement === 'left') {
    if (revealOnHover) return null;
    return { placement, chatSize, uiWidthOffset: CHAT_SEPARATOR, uiHeightOffset: 0 };
  }
  if (placement === 'bottom') {
    return { placement, chatSize, uiWidthOffset: 0, uiHeightOffset: CHAT_SEPARATOR };
  }
  return null;
}
