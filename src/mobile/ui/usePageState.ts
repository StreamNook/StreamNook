// Whether the page is seen at all and whether it is the PiP window: what
// decides if a chat list is worth drawing. Follows visibilitychange and sn:pip
// (MainActivity), and keeps the same object while nothing changed so a
// consumer re-renders only on a real flip.
import { useEffect, useState } from 'react';
import { isInPip } from '../nativeBridge';

export interface PageState {
  visible: boolean;
  pip: boolean;
}

function readPageState(): PageState {
  return {
    visible: document.visibilityState === 'visible',
    pip: isInPip() === true || document.documentElement.dataset.snPip === 'true',
  };
}

export function usePageState(): PageState {
  const [state, setState] = useState<PageState>(readPageState);
  useEffect(() => {
    const update = () =>
      setState((prev) => {
        const next = readPageState();
        return prev.visible === next.visible && prev.pip === next.pip ? prev : next;
      });
    document.addEventListener('visibilitychange', update);
    window.addEventListener('sn:pip', update);
    return () => {
      document.removeEventListener('visibilitychange', update);
      window.removeEventListener('sn:pip', update);
    };
  }, []);
  return state;
}
