// @vitest-environment node
import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';

// Every way to pause the main player ends at the same controller: Plyr's
// togglePlay on the one registered player adapter, or the <video> element's
// own pause, whose onPause reports to the earning gate and whose listener
// arms the park (VideoPlayer.tsx).

vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ currentStream: { user_login: 'x' }, currentMediaType: 'live' }) },
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

describe('pause routes', () => {
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

  it('the command palette row calls the same adapter', () => {
    const src = readFileSync('src/utils/commandPaletteSources.ts', 'utf8');
    expect(src).toContain('const pc = () => getPlayerControls();');
    expect(src).toMatch(/id: 'player\.playPause'[^\n]*run: \(\) => pc\(\)\?\.togglePlay\(\)/);
  });

  it('the adapter, the video click and the element pause reach the controller', () => {
    const src = readFileSync('src/components/VideoPlayer.tsx', 'utf8');
    // Hotkey and palette: Plyr's togglePlay on the live player.
    expect(src).toContain('togglePlay: () => playerRef.current?.togglePlay(),');
    // A single click on the video.
    expect(src).toMatch(/clickTimer = null;\s*playerRef\.current\?\.togglePlay\(\);/);
    // Every pause of the element (Plyr button, click, hotkey, palette, the
    // picture-in-picture window, media keys) reports to the earning gate...
    expect(src).toMatch(/onPause=\{\(\) => \{[\s\S]{0,400}invoke\('report_player_state', \{ state: 'paused' \}\)/);
    // ...and arms the park.
    expect(src).toContain('pauseParkRef.current?.paused();');
  });
});
