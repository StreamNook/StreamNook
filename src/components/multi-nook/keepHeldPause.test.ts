import { afterEach, describe, expect, it } from 'vitest';
import { clearPauseIntent, heldPause, notePauseIntent } from '../../utils/pausePark';
import { isViewerPlay, shouldKeepHeldPause, VIEWER_PLAY_WINDOW_MS } from './keepHeldPause';

const TILE = 'tile-keep-held';

// A stand-in for the tile's video element and its play handler, wired the way
// the hook wires them: the handler pauses the element again while a held pause
// is kept, and otherwise clears the intent.
function makeTile() {
  const state = { paused: true, playStarted: false, gestureAt: 0 };
  const onPlay = (now: number) => {
    state.paused = false;
    const held = heldPause(TILE, TILE);
    if (
      shouldKeepHeldPause({
        playStarted: state.playStarted,
        held: !!held,
        viewerPlay: isViewerPlay(state.gestureAt, now),
      })
    ) {
      state.paused = true;
      return;
    }
    clearPauseIntent(TILE);
  };
  const startPlayback = () => {
    if (state.playStarted) return;
    state.playStarted = true;
    if (heldPause(TILE, TILE)) return;
    state.paused = false;
  };
  return { state, onPlay, startPlayback };
}

afterEach(() => clearPauseIntent(TILE));

describe('shouldKeepHeldPause', () => {
  it('keeps a held pause for a play before the start point', () => {
    expect(shouldKeepHeldPause({ playStarted: false, held: true, viewerPlay: false })).toBe(true);
  });

  it('lets a play after the start point end the pause', () => {
    expect(shouldKeepHeldPause({ playStarted: true, held: true, viewerPlay: false })).toBe(false);
  });

  it('lets the viewer own play end the pause', () => {
    expect(shouldKeepHeldPause({ playStarted: false, held: true, viewerPlay: true })).toBe(false);
  });

  it('has nothing to keep without a held pause', () => {
    expect(shouldKeepHeldPause({ playStarted: false, held: false, viewerPlay: false })).toBe(false);
  });
});

describe('isViewerPlay', () => {
  it('needs a recent press in the tile', () => {
    expect(isViewerPlay(0, 5000)).toBe(false);
    expect(isViewerPlay(4500, 5000)).toBe(true);
    expect(isViewerPlay(5000 - VIEWER_PLAY_WINDOW_MS - 1, 5000)).toBe(false);
  });
});

describe('a rebuilt tile with a held pause', () => {
  it('stays paused and keeps the intent when the element plays before the start point', () => {
    notePauseIntent(TILE, TILE);
    const tile = makeTile();
    tile.onPlay(1000);
    expect(tile.state.paused).toBe(true);
    expect(heldPause(TILE, TILE)).not.toBeNull();
    tile.startPlayback();
    expect(tile.state.paused).toBe(true);
    expect(heldPause(TILE, TILE)).not.toBeNull();
  });

  it('plays at the start point when no pause is held', () => {
    const tile = makeTile();
    tile.startPlayback();
    expect(tile.state.paused).toBe(false);
  });

  it('plays and clears the pause when the viewer presses play before the start point', () => {
    notePauseIntent(TILE, TILE);
    const tile = makeTile();
    tile.state.gestureAt = 900;
    tile.onPlay(1000);
    expect(tile.state.paused).toBe(false);
    expect(heldPause(TILE, TILE)).toBeNull();
  });
});
