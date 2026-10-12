/**
 * When the title bar tucks away, as pure decisions so they can be tested.
 *
 * Two layers, kept apart on purpose:
 *
 * - `titleBarAutoHides` is the LAYOUT mode. While it holds, nothing reserves
 *   the bar's 40px: the player, MultiNook and chat run to the top edge, and the
 *   bar overlays them when it comes back. It changes only when the viewer
 *   starts or stops watching, never on hover, so revealing the bar never moves
 *   anything underneath it.
 * - `titleBarTucked` is the momentary state inside that mode: out of sight
 *   unless the pointer brought it back or something in it is in use.
 */

export interface TitleBarAutoHideInput {
  /** `video_player.hide_title_bar_while_watching`. */
  hideWhileWatching: boolean;
  /** A stream, VOD or MultiNook grid is on screen in the main window (not Home). */
  watching: boolean;
  /** The window is in OS full screen. */
  windowFullscreen: boolean;
  /** Full screen is the stream-only kind (`useStreamOnlyFullscreen`). */
  streamOnlyFullscreen: boolean;
}

export const titleBarAutoHides = ({
  hideWhileWatching,
  watching,
  windowFullscreen,
  streamOnlyFullscreen,
}: TitleBarAutoHideInput): boolean =>
  // Full screen keeps its own rule, whatever this setting says.
  windowFullscreen ? streamOnlyFullscreen : hideWhileWatching && watching;

/**
 * "Watching" for the title bar: a stream, VOD or MultiNook grid on screen, not
 * Home. The offline room (`streamUrl === 'offline'`) has no picture to give the
 * space to, so it keeps its bar, the same rule `html[data-watching]` follows.
 */
export const watchingForTitleBar = ({
  isHomeActive,
  streamUrl,
  isMultiNookActive,
}: {
  isHomeActive: boolean;
  streamUrl: string | null | undefined;
  isMultiNookActive: boolean;
}): boolean => !isHomeActive && ((!!streamUrl && streamUrl !== 'offline') || isMultiNookActive);

/**
 * How far down from the window's top edge the pointer brings the tucked bar
 * back, in CSS pixels. Decided by position, not by what is under the pointer:
 * the outermost pixels belong to window resizing on Windows (a native window
 * above the page) and Linux (LinuxResizeEdges, above everything), so a strip
 * that only covered them would never see the pointer. This reaches past both.
 */
export const TITLE_BAR_REVEAL_ZONE_PX = 10;

/** Whether a pointer at `clientY` is close enough to the top to bring the bar back.
 *  Negative values count: a pointer leaving through the top edge reports one. */
export const pointerAtTopEdge = (clientY: number): boolean => clientY <= TITLE_BAR_REVEAL_ZONE_PX;

/**
 * Store flags that put a page, overlay or modal on screen. None of them can
 * paint over the player's full screen (they sit far below its layer), so any
 * of them opening while it is up leaves the player's full screen first.
 */
export const PAGES_OVER_PLAYER = [
  'isHomeActive',
  'isSettingsOpen',
  'isCommandPaletteOpen',
  'showLiveStreamsOverlay',
  'showMarketplaceOverlay',
  'showDropsOverlay',
  'showBadgesOverlay',
  'showEmoteSetsOverlay',
  'showWhispersOverlay',
  'showDashboardOverlay',
  'profileModalUser',
  'profileViewerUserId',
] as const;

type PageFlags = { [K in (typeof PAGES_OVER_PLAYER)[number]]?: unknown };

/** Whether something in `PAGES_OVER_PLAYER` went from closed to open between two store states. */
export const opensPageOverPlayer = (prev: PageFlags, next: PageFlags): boolean =>
  PAGES_OVER_PLAYER.some((key) => !prev[key] && !!next[key]);

/** Whether anything in `PAGES_OVER_PLAYER` is open. */
export const pageOverPlayerOpen = (state: PageFlags): boolean =>
  PAGES_OVER_PLAYER.some((key) => !!state[key]);

/**
 * Whether the bar tucks at all: the layout mode above, or the player's own full
 * screen. The player's full screen pins the player over the whole window, so the
 * bar tucks there without any layout change underneath.
 */
export const titleBarTuckMode = ({
  layoutAutoHides,
  playerFullscreen,
}: {
  layoutAutoHides: boolean;
  playerFullscreen: boolean;
}): boolean => layoutAutoHides || playerFullscreen;

export interface TitleBarTuckInput {
  /** `titleBarAutoHides` for the current state. */
  autoHides: boolean;
  /** The pointer brought the bar back and has not left it long enough to tuck. */
  revealed: boolean;
  /** Something in the bar is in use: a menu or the island open, keyboard
   *  focus inside it, or a window drag under way. */
  inUse: boolean;
}

export const titleBarTucked = ({ autoHides, revealed, inUse }: TitleBarTuckInput): boolean =>
  autoHides && !revealed && !inUse;

/**
 * Whether the bar draws above the player's own layers instead of at its
 * resting layer.
 *
 * The player's top-right buttons and its title overlays sit at the bar's own
 * z-index and come later in the page, so wherever the two overlap the player
 * wins both the paint and the click. While the bar reserves its row that never
 * shows: the player starts below it. In the tucking modes nothing reserves the
 * row, so the bar lifts for as long as the mode lasts. Out, every one of its
 * controls takes the click and nothing of the player's is reachable through
 * it; tucked, it is out of sight and takes no pointer, so the player's buttons
 * work as before.
 *
 * A page, overlay or modal open over the stream keeps the bar at its resting
 * layer, under that page's backdrop, exactly where it sits whenever it
 * reserves its row. The player's own full screen always lifts it: anything
 * that opens a page there leaves full screen first.
 */
export const titleBarLifted = ({
  tuckMode,
  playerFullscreen,
  pageOpen,
}: {
  /** `titleBarTuckMode` for the current state. */
  tuckMode: boolean;
  playerFullscreen: boolean;
  /** A page, overlay or modal is open over the stream (`pageOverPlayerOpen`, or one the bar owns). */
  pageOpen: boolean;
}): boolean => playerFullscreen || (tuckMode && !pageOpen);

/** A double-click on the bar's empty space maximizes the window, except in
 *  either full screen, where there is no window size to toggle. */
export const shouldDoubleClickMaximize = ({ fullscreen }: { fullscreen: boolean }): boolean => !fullscreen;

/** A press on the bar's empty space hands the window to the OS move loop,
 *  except in either full screen: the window fills its monitor there, and a
 *  move would pull it off its edge (Windows only snaps it back when the
 *  monitor changes). */
export const shouldStartWindowDrag = ({ fullscreen }: { fullscreen: boolean }): boolean => !fullscreen;

/**
 * Whether a click in the title bar (or a menu it opened) leaves the player's
 * full screen before it runs. Anything that changes what is on screen does:
 * the page it opens would otherwise sit under the full-screen video. Opening
 * one of the bar's own menus does not (they paint over the player), and
 * neither do minimize, maximize and close, which act on the window.
 */
export const barClickExitsPlayerFullscreen = ({
  playerFullscreen,
  actionable,
  opensMenu,
  keepsFullscreen,
}: {
  playerFullscreen: boolean;
  /** The click landed on a control (button, link, menu item), not empty bar. */
  actionable: boolean;
  /** That control opens or closes one of the bar's menus (aria-expanded / aria-haspopup). */
  opensMenu: boolean;
  /** That control sits in a group marked `data-keeps-player-fullscreen`. */
  keepsFullscreen: boolean;
}): boolean => playerFullscreen && actionable && !opensMenu && !keepsFullscreen;

/** The bar's height while it reserves its row, in CSS pixels. */
export const TITLE_BAR_HEIGHT_PX = 40;

/**
 * The title bar height the aspect-ratio lock counts as chrome outside the video.
 *
 * Follows the setting rather than the live layout: the lock only matters while
 * watching, and keying it to Home vs watch would resize the window by 40px each
 * time the viewer stepped out to Home and back.
 */
export const aspectTitleBarReserve = (hideWhileWatching: boolean): number =>
  hideWhileWatching ? 0 : TITLE_BAR_HEIGHT_PX;
