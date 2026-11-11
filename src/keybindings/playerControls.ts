// Player control bridge.
//
// VideoPlayer registers an adapter over its live Plyr instance here on mount and
// clears it on unmount. Player-context keybindings (mute, fullscreen, seek, ...)
// call through this adapter so the keybinding engine never has to import Plyr or
// reach into VideoPlayer's refs. `isActive()` reports whether a real player is
// mounted (false for native-control MP4 clips), which gates the `player` context.

export interface PlayerControls {
  /** True when a controllable (Plyr) player is mounted. */
  isActive(): boolean;
  /** Current playback position in seconds, or null if unavailable. Used to
   *  anchor a VOD clip at the moment the viewer is watching. */
  getCurrentTime(): number | null;
  togglePlay(): void;
  toggleMute(): void;
  toggleFullscreen(): void;
  volumeUp(): void;
  volumeDown(): void;
  seekForward(): void;
  seekBackward(): void;
  togglePip(): void;
  speedUp(): void;
  speedDown(): void;
  /** Jump to the live edge, or back to live from the broadcast recording. */
  goLive(): void;
}

let current: PlayerControls | null = null;

export function registerPlayerControls(controls: PlayerControls | null): void {
  current = controls;
}

export function getPlayerControls(): PlayerControls | null {
  return current;
}

// The phone's player has no Plyr adapter; it registers a reader of its own
// <video>'s playhead for as long as it is mounted.
let phoneClock: (() => number | null) | null = null;

/** The phone's player: its playhead reader while it is mounted. Returns the
 *  unregister, which leaves a newer player's reader alone. */
export function registerPlayheadClock(reader: () => number | null): () => void {
  phoneClock = reader;
  return () => {
    if (phoneClock === reader) phoneClock = null;
  };
}

/** The watched player's playhead in seconds (the phone's player while it is
 *  mounted, else the desktop player's), or null when unknown. A VOD reloaded
 *  for a quality or view change starts here. */
export function playheadSecs(): number | null {
  const t = phoneClock ? phoneClock() : (current?.getCurrentTime() ?? null);
  return t == null || !Number.isFinite(t) ? null : t;
}

/** True when a real player is mounted and accepting control. */
export function isPlayerControllable(): boolean {
  return current !== null && current.isActive();
}
