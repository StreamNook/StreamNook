// Whether a chat's pin stack is actually in front of the viewer (SN-284). A
// pin's time counts only while it is on screen, and "drawn" is not enough: the
// chat can be mounted under the Home veil, a settings page, a full-screen
// player without its chat column, a collapsed hover-reveal column, a sheet. The
// pins' observer would still see them intersecting the viewport, so the shell
// says so itself, from one pure rule (unit tested) both platforms use.

/** What a desktop chat surface knows about the window it sits in. */
export interface DesktopShownInputs {
  platform: 'desktop';
  /** `document.visibilityState === 'visible'`. */
  pageVisible: boolean;
  /** The Home screen veils the stream view (and its chat). */
  homeActive: boolean;
  settingsOpen: boolean;
  badgesOpen: boolean;
  whispersOpen: boolean;
  /** A user's profile card is open as a modal. */
  profileModalOpen: boolean;
  /** The player is full screen and the chat column is not up over it. */
  fullScreenWithoutChat: boolean;
  /** Hover-reveal chat (auto-hide) is collapsed to its edge handle. */
  hoverRevealCollapsed: boolean;
  /** MultiNook's chat is hidden. */
  multiNookChatHidden: boolean;
  /** This chat's own view is the chat (not Viewers, Mod Room, ...). */
  chatViewActive: boolean;
  /** Chat search is open: the stack is hidden under it. */
  searchOpen: boolean;
  /** The drops drawer or the emote picker covers the stack. */
  coverOpen: boolean;
}

/** What an Android chat surface knows. */
export interface AndroidShownInputs {
  platform: 'android';
  pageVisible: boolean;
  /** The chat is scrolled or tucked out of the screen (landscape chat off, a
   *  collapsed panel, the Chats tab not on top). */
  chatOutOfSight: boolean;
  /** A sheet or a pushed screen is over the chat. */
  sheetOpen: boolean;
}

export type ChatShownInputs = DesktopShownInputs | AndroidShownInputs;

export function chatSurfaceShown(i: ChatShownInputs): boolean {
  if (!i.pageVisible) return false;
  if (i.platform === 'android') return !i.chatOutOfSight && !i.sheetOpen;
  return !(
    i.homeActive ||
    i.settingsOpen ||
    i.badgesOpen ||
    i.whispersOpen ||
    i.profileModalOpen ||
    i.fullScreenWithoutChat ||
    i.hoverRevealCollapsed ||
    i.multiNookChatHidden ||
    !i.chatViewActive ||
    i.searchOpen ||
    i.coverOpen
  );
}
