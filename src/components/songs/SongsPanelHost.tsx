// The desktop's Your songs: a modal over the window, opened from anywhere
// through `openSongs` (utils/songHistory.ts). Mounted in the main window and
// in every MultiChat window; the panel itself loads only when opened. It lifts
// over the player's full screen like the other modals chat opens (globals.css
// --sn-z-over-player-lift), so a song card's link in the full-screen chat
// opens it in place. No blur on the backdrop: a full-window blur over a
// playing stream repaints every video frame, so the panel drops its own blur
// too and paints the over-video flyout's solid theme base (glass-flyout
// no-live-blur), which keeps it readable over a moving stream on light
// themes and dark ones alike.
import { lazy, Suspense, useCallback, useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { useAppStore } from '../../stores/AppStore';
import { registerSongsSurface } from '../../utils/songHistory';
import ErrorBoundary from '../ErrorBoundary';
import { songLayerOpen } from '../../utils/songLinks';

const SongsPanel = lazy(() => import('./SongsPanel'));

/** What Tab can reach inside the dialog: shown, enabled controls. */
function tabStops(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('button, input, a[href], [tabindex]:not([tabindex="-1"])')].filter(
    (el) => !el.hasAttribute('disabled') && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden',
  );
}

/** What shows when the panel fails: nothing, and the panel is closed, so the
 *  window's keys come back (they stand down while it is open). Closing it
 *  changes the boundary's key, which draws the host afresh for the next open. */
function ClosedAfterFault() {
  const closeSongs = useAppStore((s) => s.closeSongs);
  useEffect(() => closeSongs(), [closeSongs]);
  return null;
}

/** The panel behind its own boundary: a fault in it (or in a song link's QR
 *  dialog, drawn from inside it) closes it and leaves the stream and the chat
 *  running. */
export function SongsPanelHost() {
  const panel = useAppStore((s) => s.songsPanel);
  return (
    <ErrorBoundary
      componentName="Your songs"
      reportToLogService
      resetKeys={[panel]}
      fallbackRender={() => <ClosedAfterFault />}
    >
      <PanelHost />
    </ErrorBoundary>
  );
}

function PanelHost() {
  const panel = useAppStore((s) => s.songsPanel);
  const closeSongs = useAppStore((s) => s.closeSongs);
  const setSongsFocus = useAppStore((s) => s.setSongsFocus);

  useEffect(() => registerSongsSurface('panel'), []);

  // The panel's turn at Escape (it clears a typed search), registered while
  // it is mounted.
  const panelEscape = useRef<(() => boolean) | null>(null);
  const registerEscape = useCallback((handler: (() => boolean) | null) => {
    panelEscape.current = handler;
  }, []);

  // Escape steps back, and nothing under the panel hears it: a song's detail
  // to the list, a typed search to empty, the list to closed.
  const focus = panel?.focus ?? null;
  const isOpen = panel !== null;
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      // A song link's QR code or app menu over the panel takes this press.
      if (e.key !== 'Escape' || songLayerOpen(document)) return;
      e.stopPropagation();
      e.preventDefault();
      if (focus) setSongsFocus(null);
      else if (!panelEscape.current?.()) closeSongs();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [isOpen, focus, setSongsFocus, closeSongs]);

  // The panel takes the keyboard while open (typing never reaches the chat
  // box under it) and hands it back to where it was on close.
  const dialogRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!isOpen) return;
    const before = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus({ preventScroll: true });
    return () => before?.focus?.({ preventScroll: true });
  }, [isOpen]);

  // Tab goes round inside the panel instead of out to the page under it.
  const onDialogKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Tab' || !dialogRef.current) return;
    const stops = tabStops(dialogRef.current);
    if (stops.length === 0) {
      e.preventDefault();
      return;
    }
    const first = stops[0];
    const last = stops[stops.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === dialogRef.current)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <AnimatePresence>
      {panel && (
        <motion.div
          key="songs"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2, ease: [0.2, 0.8, 0.2, 1] }}
          className="fixed inset-0 z-[calc(var(--sn-z-over-player-lift,0)_+_50)] flex items-center justify-center bg-black/60"
          // A press that starts on the backdrop closes; a text selection
          // dragged out of the panel does not.
          onPointerDown={(e) => {
            if (e.button === 0 && e.target === e.currentTarget) closeSongs();
          }}
        >
          <motion.div
            ref={dialogRef}
            role="dialog"
            aria-modal="true"
            aria-label="Your songs"
            tabIndex={-1}
            onKeyDown={onDialogKeyDown}
            // Opens and closes by scale and fade only (the Motion setting keeps
            // just the fade).
            initial={{ scale: 0.97, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            exit={{ scale: 0.97, opacity: 0 }}
            transition={{ duration: 0.2, ease: [0.2, 0.8, 0.2, 1] }}
            className="glass-flyout no-live-blur flex h-[720px] max-h-[86vh] w-[600px] max-w-[calc(100vw-32px)] flex-col overflow-hidden !rounded-2xl !p-0 shadow-[0_20px_50px_rgba(0,0,0,0.5)] outline-none"
          >
            <Suspense fallback={null}>
              <SongsPanel
                variant="modal"
                focus={panel.focus}
                onFocus={setSongsFocus}
                onClose={closeSongs}
                registerEscape={registerEscape}
              />
            </Suspense>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
