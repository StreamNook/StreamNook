// `chatSurfaceShown` for a desktop chat, read from the stores that say what the
// app has put over it (SN-284). The caller adds what only its own chat knows:
// whether its view is the chat and whether its search is open.

import { useEffect, useState } from 'react';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { chatSurfaceShown } from '../utils/chatSurfaceShown';
import { useChatCovered } from '../utils/chatCovers';

/** `document.visibilityState === 'visible'`, following changes. */
export function usePageVisible(): boolean {
  const [visible, setVisible] = useState(() => typeof document === 'undefined' || document.visibilityState === 'visible');
  useEffect(() => {
    const on = () => setVisible(document.visibilityState === 'visible');
    document.addEventListener('visibilitychange', on);
    on();
    return () => document.removeEventListener('visibilitychange', on);
  }, []);
  return visible;
}

export function useDesktopChatShown(local: { chatViewActive: boolean; searchOpen: boolean }): boolean {
  const homeActive = useAppStore((s) => s.isHomeActive);
  const settingsOpen = useAppStore((s) => s.isSettingsOpen);
  const badgesOpen = useAppStore((s) => s.showBadgesOverlay);
  const whispersOpen = useAppStore((s) => s.showWhispersOverlay);
  const profileModalOpen = useAppStore((s) => s.profileModalUser != null);
  const fullScreenWithoutChat = useAppStore((s) => s.isPlayerFullscreen && !s.fsChatOpen);
  const multiNookChatHidden = usemultiNookStore((s) => s.isMultiNookActive && s.isChatHidden);
  const coverOpen = useChatCovered();
  const pageVisible = usePageVisible();
  return chatSurfaceShown({
    platform: 'desktop',
    pageVisible,
    homeActive,
    settingsOpen,
    badgesOpen,
    whispersOpen,
    profileModalOpen,
    fullScreenWithoutChat,
    // Set by App (a collapsed hover-reveal column) through chatCovers.
    hoverRevealCollapsed: false,
    multiNookChatHidden,
    chatViewActive: local.chatViewActive,
    searchOpen: local.searchOpen,
    coverOpen,
  });
}
