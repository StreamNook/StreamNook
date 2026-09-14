// Fallback keyboard-height tracking via visualViewport. The PRIMARY source is
// the native inset bridge (MainActivity writes --sn-kb from the IME inset);
// this hook only fills --sn-kb when the native value is absent, covering dev
// setups where the bridge is not yet active.
import { useEffect } from 'react';

export function useKeyboardInsets(): void {
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const root = document.documentElement;
    const update = () => {
      // Only act as fallback: the native bridge marks its ownership.
      if (root.dataset.snNativeInsets === 'true') return;
      const kb = Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop));
      root.style.setProperty('--sn-kb', `${kb}px`);
    };
    vv.addEventListener('resize', update);
    vv.addEventListener('scroll', update);
    update();
    return () => {
      vv.removeEventListener('resize', update);
      vv.removeEventListener('scroll', update);
    };
  }, []);
}
