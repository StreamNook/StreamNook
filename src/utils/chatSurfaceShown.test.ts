// SN-284: the one rule that says whether a chat's pin stack is in front of the
// viewer. Every cover the app can put over a chat makes it false.
import { describe, expect, it } from 'vitest';
import { chatSurfaceShown, type AndroidShownInputs, type DesktopShownInputs } from './chatSurfaceShown';

const desktop = (over: Partial<DesktopShownInputs> = {}): DesktopShownInputs => ({
  platform: 'desktop',
  pageVisible: true,
  homeActive: false,
  settingsOpen: false,
  badgesOpen: false,
  whispersOpen: false,
  profileModalOpen: false,
  fullScreenWithoutChat: false,
  hoverRevealCollapsed: false,
  multiNookChatHidden: false,
  chatViewActive: true,
  searchOpen: false,
  coverOpen: false,
  ...over,
});

const android = (over: Partial<AndroidShownInputs> = {}): AndroidShownInputs => ({
  platform: 'android',
  pageVisible: true,
  chatOutOfSight: false,
  sheetOpen: false,
  ...over,
});

describe('chatSurfaceShown (desktop)', () => {
  it('is true for a chat that nothing covers', () => {
    expect(chatSurfaceShown(desktop())).toBe(true);
  });

  it.each([
    ['the page is hidden', { pageVisible: false }],
    ['the Home veil is up', { homeActive: true }],
    ['Settings is open', { settingsOpen: true }],
    ['the badges overlay is open', { badgesOpen: true }],
    ['the whispers overlay is open', { whispersOpen: true }],
    ['a profile modal is open', { profileModalOpen: true }],
    ['full screen has no chat column up', { fullScreenWithoutChat: true }],
    ['hover-reveal chat is collapsed', { hoverRevealCollapsed: true }],
    ['MultiNook hides its chat', { multiNookChatHidden: true }],
    ['the chat shows another view (Viewers, Mod Room)', { chatViewActive: false }],
    ['chat search is open', { searchOpen: true }],
    ['the drops drawer or emote picker is over the stack', { coverOpen: true }],
  ] as Array<[string, Partial<DesktopShownInputs>]>)('is false when %s', (_name, over) => {
    expect(chatSurfaceShown(desktop(over))).toBe(false);
  });
});

describe('chatSurfaceShown (Android)', () => {
  it('is true for a chat in sight with no sheet over it', () => {
    expect(chatSurfaceShown(android())).toBe(true);
  });

  it('is false when the page is hidden, the chat is out of sight or a sheet is open', () => {
    expect(chatSurfaceShown(android({ pageVisible: false }))).toBe(false);
    expect(chatSurfaceShown(android({ chatOutOfSight: true }))).toBe(false);
    expect(chatSurfaceShown(android({ sheetOpen: true }))).toBe(false);
  });
});
