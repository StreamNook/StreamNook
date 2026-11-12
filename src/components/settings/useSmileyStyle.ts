// The Smiley style row's state. Reads the saved view once at mount (it answers
// from Rust's record, never the network), draws the row only if that first
// answer has a view, then follows the views Rust sends through the chat store's
// single listener. A tap runs through `handleTile`.
import { useCallback, useEffect, useReducer } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getOwnSmilies, subscribeOwnSmilies } from '../../stores/chatConnectionStore';
import { useAppStore } from '../../stores/AppStore';
import { openExternal } from '../../utils/openExternal';
import {
  INITIAL_RING,
  handleTile,
  ringReducer,
  type SmileyStyle,
  type SmileyStyleRead,
} from './smileyStyle';

export function useSmileyStyle(): { view: SmileyStyle | null; pending: string | null; select: (key: string) => void } {
  const [ring, dispatch] = useReducer(ringReducer, INITIAL_RING);

  useEffect(() => {
    let live = true;
    // The newest rev this mount has taken, so an older view never replaces a newer one.
    let seen = -1;
    invoke<SmileyStyleRead>('get_smiley_style')
      .then((answer) => {
        if (!live) return;
        // A view that reached the store while this answer was on its way is the newer word.
        const latest = getOwnSmilies();
        seen = Math.max(answer.rev, latest.rev);
        dispatch({ type: 'load', view: latest.rev > answer.rev ? latest.view : answer.view });
      })
      .catch(() => {
        if (live) dispatch({ type: 'load', view: null });
      });
    const off = subscribeOwnSmilies(() => {
      const latest = getOwnSmilies();
      if (latest.rev <= seen) return;
      seen = latest.rev;
      dispatch({ type: 'event', view: latest.view });
    });
    return () => {
      live = false;
      off();
    };
  }, []);

  const select = useCallback(
    (key: string) => {
      if (!ring.view) return;
      void handleTile(ring.view, key, ring.pending, {
        dispatch,
        setStyle: (k) => invoke<SmileyStyle>('set_smiley_style', { key: k }),
        open: openExternal,
        toast: (message) => useAppStore.getState().addToast(message, 'error'),
      });
    },
    [ring.view, ring.pending],
  );

  return { view: ring.view, pending: ring.pending, select };
}
