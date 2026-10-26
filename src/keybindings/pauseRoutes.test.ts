// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Every way to pause the main player ends at the same controller: Plyr's
// togglePlay on the one registered player adapter, or the <video> element's
// own `pause` event, which reports to the earning gate and arms the park.

vi.mock('../stores/AppStore', () => ({
  useAppStore: {
    getState: () => ({
      currentStream: { user_login: 'x' },
      currentMediaType: 'live',
      followedStreams: [],
      currentUser: null,
      settings: {},
    }),
  },
}));
vi.mock('../stores/multiNookStore', () => ({ usemultiNookStore: { getState: () => ({}) } }));
vi.mock('../stores/chatDockStore', () => ({
  chatKey: vi.fn(),
  dockView: vi.fn(),
  setDockPanelOpen: vi.fn(),
  showDockedChat: vi.fn(),
  useChatDockStore: { getState: () => ({}) },
}));
vi.mock('./chatModController', () => ({ getChatModController: () => null }));
vi.mock('./chatSearchController', () => ({ getChatSearchController: () => null }));
vi.mock('./paneFocusController', () => ({ getPaneFocusController: () => null }));
vi.mock('../utils/windowFullscreen', () => ({ exitPlayerFullscreen: vi.fn() }));
vi.mock('../utils/platform', () => ({ IS_MAC: false, IS_MOBILE: false }));

const { registerPlayerControls } = await import('./playerControls');
const { getBindableCommand } = await import('./commands');
const { attachPauseRoutes, CLICK_TOGGLE_DELAY_MS, releaseTypingFocus } = await import('../utils/pauseRoutes');
const { getStaticItems } = await import('../utils/commandPaletteSources');

describe('pause routes', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('the hotkey command pauses through the registered player', () => {
    const togglePlay = vi.fn();
    registerPlayerControls({
      isActive: () => true,
      getCurrentTime: () => 0,
      togglePlay,
      toggleMute: vi.fn(),
      toggleFullscreen: vi.fn(),
      volumeUp: vi.fn(),
      volumeDown: vi.fn(),
      seekForward: vi.fn(),
      seekBackward: vi.fn(),
      togglePip: vi.fn(),
      speedUp: vi.fn(),
      speedDown: vi.fn(),
      goLive: vi.fn(),
    });
    void getBindableCommand('player.playPause')?.run?.();
    expect(togglePlay).toHaveBeenCalledTimes(1);
    registerPlayerControls(null);
  });

  const wire = () => {
    const video = new EventTarget();
    const deps = {
      togglePlay: vi.fn(),
      reportPaused: vi.fn(),
      parkPaused: vi.fn(),
    };
    const detach = attachPauseRoutes(video as unknown as HTMLVideoElement, deps);
    return { video, deps, detach };
  };

  it('a pause of the element reports to the earning gate and reaches the park', () => {
    // The Plyr button, picture-in-picture, media keys, the hotkey and the
    // palette all pause the element, so they all end in this event.
    const { video, deps } = wire();
    video.dispatchEvent(new Event('pause'));
    expect(deps.reportPaused).toHaveBeenCalledTimes(1);
    expect(deps.parkPaused).toHaveBeenCalledTimes(1);
    expect(deps.togglePlay).not.toHaveBeenCalled();
  });

  it('a single click on the video toggles play after the click delay', () => {
    const { video, deps } = wire();
    video.dispatchEvent(new Event('click'));
    expect(deps.togglePlay).not.toHaveBeenCalled();
    vi.advanceTimersByTime(CLICK_TOGGLE_DELAY_MS);
    expect(deps.togglePlay).toHaveBeenCalledTimes(1);
  });

  it('a double click cancels the pending click and never toggles play', () => {
    // Fullscreen on a double click is Plyr's own; the route only keeps the
    // click from turning into a play/pause blip.
    const { video, deps } = wire();
    video.dispatchEvent(new Event('click'));
    video.dispatchEvent(new Event('click'));
    video.dispatchEvent(new Event('dblclick'));
    vi.advanceTimersByTime(CLICK_TOGGLE_DELAY_MS * 2);
    expect(deps.togglePlay).not.toHaveBeenCalled();
    expect(deps.reportPaused).not.toHaveBeenCalled();
    expect(deps.parkPaused).not.toHaveBeenCalled();
  });

  it('the palette row pauses through the registered player', () => {
    const togglePlay = vi.fn();
    registerPlayerControls({
      isActive: () => true,
      getCurrentTime: () => 0,
      togglePlay,
      toggleMute: vi.fn(),
      toggleFullscreen: vi.fn(),
      volumeUp: vi.fn(),
      volumeDown: vi.fn(),
      seekForward: vi.fn(),
      seekBackward: vi.fn(),
      togglePip: vi.fn(),
      speedUp: vi.fn(),
      speedDown: vi.fn(),
      goLive: vi.fn(),
    });
    const row = getStaticItems().find((item) => item.id === 'player.playPause');
    expect(row).toBeDefined();
    void row?.run();
    expect(togglePlay).toHaveBeenCalledTimes(1);
    registerPlayerControls(null);
  });

  it('with Click to pause off, a click releases the text box and never toggles', () => {
    const video = new EventTarget();
    let clickToPause = false;
    const deps = {
      togglePlay: vi.fn(),
      reportPaused: vi.fn(),
      parkPaused: vi.fn(),
      clickToPause: () => clickToPause,
      releaseFocus: vi.fn(),
    };
    attachPauseRoutes(video as unknown as HTMLVideoElement, deps);
    video.dispatchEvent(new Event('click'));
    vi.advanceTimersByTime(CLICK_TOGGLE_DELAY_MS * 2);
    expect(deps.releaseFocus).toHaveBeenCalledTimes(1);
    expect(deps.togglePlay).not.toHaveBeenCalled();
    // The setting is read at click time: turned back on, the next click toggles.
    clickToPause = true;
    video.dispatchEvent(new Event('click'));
    vi.advanceTimersByTime(CLICK_TOGGLE_DELAY_MS);
    expect(deps.releaseFocus).toHaveBeenCalledTimes(2);
    expect(deps.togglePlay).toHaveBeenCalledTimes(1);
  });

  it('with Click to pause off, a pause from the button or a key still parks the download', () => {
    const video = new EventTarget();
    const deps = {
      togglePlay: vi.fn(),
      reportPaused: vi.fn(),
      parkPaused: vi.fn(),
      clickToPause: () => false,
      releaseFocus: vi.fn(),
    };
    attachPauseRoutes(video as unknown as HTMLVideoElement, deps);
    video.dispatchEvent(new Event('pause'));
    expect(deps.reportPaused).toHaveBeenCalledTimes(1);
    expect(deps.parkPaused).toHaveBeenCalledTimes(1);
  });

  it('releaseTypingFocus blurs a text box and leaves anything else focused', () => {
    const typing = { tagName: 'TEXTAREA', blur: vi.fn() };
    releaseTypingFocus({ activeElement: typing as unknown as Element });
    expect(typing.blur).toHaveBeenCalledTimes(1);
    const editable = { tagName: 'DIV', isContentEditable: true, blur: vi.fn() };
    releaseTypingFocus({ activeElement: editable as unknown as Element });
    expect(editable.blur).toHaveBeenCalledTimes(1);
    const button = { tagName: 'BUTTON', isContentEditable: false, blur: vi.fn() };
    releaseTypingFocus({ activeElement: button as unknown as Element });
    expect(button.blur).not.toHaveBeenCalled();
    releaseTypingFocus({ activeElement: null });
  });

  it('detaching removes every route and drops a pending click', () => {
    const { video, deps, detach } = wire();
    video.dispatchEvent(new Event('click'));
    detach();
    vi.advanceTimersByTime(CLICK_TOGGLE_DELAY_MS * 2);
    video.dispatchEvent(new Event('click'));
    video.dispatchEvent(new Event('pause'));
    vi.advanceTimersByTime(CLICK_TOGGLE_DELAY_MS * 2);
    expect(deps.togglePlay).not.toHaveBeenCalled();
    expect(deps.reportPaused).not.toHaveBeenCalled();
    expect(deps.parkPaused).not.toHaveBeenCalled();
  });
});
