// The emote the click card is showing, per window (each Tauri window has its
// own store, so a click in a popout opens the card in that popout).
import { create } from 'zustand';
import { useTooltipStore } from './TooltipStore';

export interface EmoteCardTarget {
  provider: string;
  id: string;
  /** The name it was shown under in chat. */
  name: string;
  /** The image chat drew, the fallback when the large one fails. */
  url: string;
  channelLogin?: string | null;
  channelId?: string | null;
}

interface EmoteCardState {
  emote: EmoteCardTarget | null;
  /** True where the 7TV emote spotlight (with its add-to-channel tools) is
   *  mounted; elsewhere a 7TV emote opens this card instead. */
  seventvSpotlight: boolean;
  open: (emote: EmoteCardTarget) => void;
  close: () => void;
  setSeventvSpotlight: (mounted: boolean) => void;
}

export const useEmoteCardStore = create<EmoteCardState>((set) => ({
  emote: null,
  seventvSpotlight: false,
  open: (emote) => set({ emote }),
  close: () => set({ emote: null }),
  setSeventvSpotlight: (mounted) => set({ seventvSpotlight: mounted }),
}));

/** Show `emote` in this window's card; the hover card under the pointer goes
 *  first, since the modal covers its trigger and no mouseleave would close it. */
export function openEmoteCard(emote: EmoteCardTarget): void {
  useTooltipStore.getState().hideTooltip();
  useEmoteCardStore.getState().open(emote);
}
