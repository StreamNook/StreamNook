import { useAppStore } from '../stores/AppStore';
import { confettiClearMs } from '../utils/confettiClear';

/** The live Confetti cleanup setting, for the chats that throw hype train confetti. */
export function useConfettiClearMs(): number | null {
  return useAppStore((s) => confettiClearMs(s.settings.chat_design));
}
