import { useEffect } from 'react';

import { useContextMenuStore } from '../stores/contextMenuStore';
import { StreamContextMenu } from './StreamContextMenu';

/**
 * Replaces the webview's context menu with StreamNook's own, everywhere.
 *
 * Right-clicking an input opens the editing menu (cut/copy/paste plus spelling
 * corrections); right-clicking a text selection offers Copy; everything else
 * gets nothing, which is what a desktop app window should do.
 *
 * Mounted in BOTH roots. MultiChat popouts render MultiChatWindow rather than
 * App (see main.tsx), so leaving this in App alone meant popout composers fell
 * through to the OS menu — a different menu, in the surface a MultiChat user
 * types in most.
 */
export const InputContextMenuHost: React.FC = () => {
  useEffect(() => {
    const handleContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      const isInput =
        target.tagName === 'INPUT' ||
        target.tagName === 'TEXTAREA' ||
        target.isContentEditable ||
        target.closest('input, textarea, [contenteditable]');

      if (isInput) {
        e.preventDefault();
        useContextMenuStore.getState().openInputMenu(e, target as HTMLElement);
        return;
      }

      const selection = window.getSelection();
      if (selection && selection.toString().trim().length > 0) {
        e.preventDefault();
        useContextMenuStore.getState().openSelectionMenu(e);
        return;
      }

      e.preventDefault();
    };

    document.addEventListener('contextmenu', handleContextMenu);
    return () => document.removeEventListener('contextmenu', handleContextMenu);
  }, []);

  return <StreamContextMenu />;
};

export default InputContextMenuHost;
