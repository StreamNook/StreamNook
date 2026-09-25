import { useEffect, useState } from 'react';
import { Window } from '@tauri-apps/api/window';
import { Minus, X, CornersOut, CornersIn } from 'phosphor-react';

import { IS_MAC, IS_MOBILE } from '../../utils/platform';
import { Tooltip } from '../ui/Tooltip';

/**
 * Minimize, maximize and close, for full-window surfaces that cover the title
 * bar (the setup wizard).
 *
 * The window has no OS frame on Windows and Linux, so without these a surface
 * that hides the title bar leaves no visible way to move, shrink or close the
 * window. On GNOME and tiling Wayland compositors there is no taskbar button to
 * fall back on either. Same classes and geometry as the title bar's own
 * cluster, so the corner reads the same on every screen.
 *
 * Renders nothing on macOS, where AppKit draws the traffic lights, and on
 * mobile, which has no window.
 */
export function WindowCaptionButtons({ className = '' }: { className?: string }) {
  const [isMaximized, setIsMaximized] = useState(false);
  const hidden = IS_MAC || IS_MOBILE;

  useEffect(() => {
    if (hidden) return;
    const win = Window.getCurrent();
    const refresh = () => {
      win.isMaximized().then(setIsMaximized).catch(() => {});
    };
    refresh();
    const unlisten = win.onResized(refresh);
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [hidden]);

  if (hidden) return null;

  return (
    <div className={`flex items-center titlebar-window-controls ${className}`}>
      <Tooltip content="Minimize" delay={200}>
        <button
          onClick={() => void Window.getCurrent().minimize()}
          className="titlebar-window-btn"
          aria-label="Minimize"
        >
          <Minus size={14} />
        </button>
      </Tooltip>
      <Tooltip content={isMaximized ? 'Restore' : 'Maximize'} delay={200}>
        <button
          onClick={() => void Window.getCurrent().toggleMaximize()}
          className="titlebar-window-btn"
          aria-label={isMaximized ? 'Restore' : 'Maximize'}
        >
          {isMaximized ? <CornersIn size={14} /> : <CornersOut size={14} />}
        </button>
      </Tooltip>
      <Tooltip content="Close" delay={200}>
        <button
          onClick={() => void Window.getCurrent().close()}
          className="titlebar-window-btn titlebar-window-btn-close"
          aria-label="Close"
        >
          <X size={14} />
        </button>
      </Tooltip>
    </div>
  );
}
