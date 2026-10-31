// Which card was held, and what its tap does. One hold sheet serves every
// card (mounted once in MobileApp), so a card that moves after a press (a
// favorite leaving the list for the shelf) or unmounts never takes the sheet
// with it.
import { create } from 'zustand';
import type { TwitchStream } from '../../types';

export interface HeldChannel {
  stream: TwitchStream;
  /** The held card's own tap: Watch does what a tap on that card does. */
  onPress: (stream: TwitchStream) => void;
}

interface ChannelHoldSheetState {
  held: HeldChannel | null;
  open: (held: HeldChannel) => void;
  close: () => void;
}

export const useChannelHoldSheet = create<ChannelHoldSheetState>((set) => ({
  held: null,
  open: (held) => set({ held }),
  close: () => set({ held: null }),
}));
