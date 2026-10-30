// Where a paused phone chat's overlay sits (ResumeLiveButton.tsx draws it).
import type React from 'react';
import { useEffect, useState } from 'react';
import { useMentionsStore, type InboxSurface } from '../mentions/mentionInbox';

/**
 * How far the strip the composer raises over the chat (the emote suggestions
 * while typing) reaches into the bottom of the list, so what sits there can
 * clear it. Zero while nothing rises. Read from this pane's own DOM, so a
 * second chat (Chats under the player) never takes another composer's strip.
 */
export function useComposerRiserLift(
  rootRef: React.RefObject<HTMLElement | null>,
  listRef: React.RefObject<HTMLElement | null>,
  enabled: boolean,
  /** This pane's chat: only its own composer's riser lifts it. */
  surface: InboxSurface,
): number {
  const busy = useMentionsStore((s) => s.composerBusy[surface]);
  const [lift, setLift] = useState(0);
  const watching = enabled && busy;
  useEffect(() => {
    const root = rootRef.current;
    const list = listRef.current;
    if (!watching || !root || !list) return;
    const riser = root.querySelector<HTMLElement>('[data-composer-riser]');
    if (!riser) return;
    const measure = () => {
      // Layout offsets, not the riser's own rect: it rises in with a small
      // transform, and the resting place is what to clear.
      const parent = riser.offsetParent as HTMLElement | null;
      const top = (parent ? parent.getBoundingClientRect().top : 0) + riser.offsetTop;
      setLift(Math.max(0, Math.round(list.getBoundingClientRect().bottom - top)));
    };
    const frame = requestAnimationFrame(measure);
    const observer = new ResizeObserver(measure);
    observer.observe(riser);
    observer.observe(list);
    const composer = root.querySelector<HTMLElement>('[data-chat-composer]');
    if (composer) observer.observe(composer);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [watching, rootRef, listRef]);
  return watching ? lift : 0;
}
