// Whether the pinned banner of the chat on screen is unrolled, shared by the
// watch screen, the Chats screen and the composer menu. Each room starts as
// Chat > Pinned Messages > Collapsed says; a tap holds until the chat switches.
// With Style set to Hidden a folded pin draws nothing over chat, so the
// composer menu is the way to open it (the desktop keeps a pin in its chat
// header for the same job).
import { create } from 'zustand';
import { useAppStore } from '../../stores/AppStore';
import { usePinStore } from '../../stores/pinStore';
import { useActiveChatChannelId } from '../chat/ChatTabStrip';

interface PinsOpenState {
  /** The last room a tap opened or folded, and which way. */
  choice: { channel: string | null; open: boolean } | null;
  setOpen: (channel: string | null, open: boolean) => void;
}

export const usePinsOpenStore = create<PinsOpenState>((set) => ({
  choice: null,
  setOpen: (channel, open) => set({ choice: { channel, open } }),
}));

/** Whether a room's banner is open: the room's own tap, else the setting. */
export function pinsOpenFor(
  choice: PinsOpenState['choice'],
  channel: string | null,
  startCollapsed: boolean,
): boolean {
  return choice && choice.channel === channel ? choice.open : !startCollapsed;
}

/** The chat on screen's banner: open or folded, how a folded one draws, and
 *  the toggle a tap on it sends. */
export function usePinsOpen(): {
  open: boolean;
  style: 'bar' | 'hidden';
  setOpen: (open: boolean) => void;
} {
  const channel = useActiveChatChannelId();
  const startCollapsed = useAppStore((s) => s.settings.chat_design?.pinned_start_collapsed) ?? true;
  const style = useAppStore((s) => s.settings.chat_design?.pinned_collapsed_style) ?? 'bar';
  const choice = usePinsOpenStore((s) => s.choice);
  const setChoice = usePinsOpenStore((s) => s.setOpen);
  return {
    open: pinsOpenFor(choice, channel, startCollapsed),
    style,
    setOpen: (open) => setChoice(channel, open),
  };
}

/** Whether the composer menu offers "Pinned message": the room on screen has a
 *  pin and Style keeps it out of chat while it is folded. */
export function useHiddenPinWaiting(): boolean {
  const { open, style } = usePinsOpen();
  const hasPins = usePinStore((s) => s.pinnedIds.length > 0);
  return hasPins && style === 'hidden' && !open;
}
