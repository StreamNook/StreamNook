import { useAppStore } from '../stores/AppStore';
import { getShortcutDisplayMap } from '../keybindings/registry';
import { injectPlyrControl } from './plyrControls';

/**
 * The chat button in a Plyr control bar: brings chat up over the player's
 * full screen and puts it away again, without leaving full screen. Shared by
 * the solo player and every MultiNook tile. CSS shows it only in the player's
 * own full screen while there is a chat to bring up (globals.css,
 * html[data-fs-chat-available]); the window and Theater mode have chat docked
 * already. It sits just before the full screen button.
 */
export const FULLSCREEN_CHAT_ATTR = 'data-streamnook-fschat';

const CHAT_BUTTON_HTML = `
  <svg class="plyr__icon" aria-hidden="true" focusable="false" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
  </svg>
  <span class="plyr__tooltip" role="tooltip">Chat</span>
`;

/** Light the button while chat is up, and name what a click does next. */
export function paintFullscreenChatButton(btn: Element | null, open: boolean): void {
  if (!btn) return;
  btn.classList.toggle('is-active', open);
  btn.setAttribute('aria-pressed', open ? 'true' : 'false');
  const label = open ? 'Hide chat' : 'Chat';
  btn.setAttribute('aria-label', label);
  const key = getShortcutDisplayMap()['player.fullscreenChat'];
  const tip = btn.querySelector('.plyr__tooltip');
  if (tip) tip.textContent = key ? `${label} (${key})` : label;
}

/** Insert the chat button into `container`'s control bar and keep it painted.
 *  Returns the cleanup. */
export function mountFullscreenChatButton(container: HTMLElement): () => void {
  const cancel = injectPlyrControl(container, {
    attr: FULLSCREEN_CHAT_ATTR,
    className: '',
    html: CHAT_BUTTON_HTML,
    onClick: () => useAppStore.getState().toggleFsChat(),
    place: (controls) => {
      const fullscreen = controls.querySelector(':scope > [data-plyr="fullscreen"]');
      return fullscreen ? { before: fullscreen } : null;
    },
    onInserted: (btn) => paintFullscreenChatButton(btn, useAppStore.getState().fsChatOpen),
  });
  const unsubscribe = useAppStore.subscribe((s, prev) => {
    if (s.fsChatOpen === prev.fsChatOpen) return;
    paintFullscreenChatButton(container.querySelector(`[${FULLSCREEN_CHAT_ATTR}]`), s.fsChatOpen);
  });
  return () => {
    cancel();
    unsubscribe();
  };
}
