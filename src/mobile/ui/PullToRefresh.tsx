// Pull-to-refresh: the standard mobile refresh idiom. Native touch listeners
// with a non-passive touchmove, because Android's scroll gesture claims pointer
// events (pointercancel fires as soon as the browser takes the scroll), which
// silently killed the first pointer-event implementation. While the list sits
// at its top and the finger drags down, the touchmove is preventDefault()ed
// and drives the pull; everywhere else the browser scrolls normally.
// Indicator: a down arrow fades/scales in with the drag and flips upward once
// past the release threshold, then a spinner runs while onRefresh resolves.
import React, { useEffect, useRef, useState } from 'react';
import { ArrowDown } from 'phosphor-react';
import PenroseMarch from '../../components/PenroseMarch';

const ARM_THRESHOLD_PX = 72;
const MAX_PULL_PX = 120;

export const PullToRefresh: React.FC<{
  onRefresh: () => Promise<unknown>;
  className?: string;
  children: React.ReactNode;
}> = ({ onRefresh, className, children }) => {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [pull, setPull] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  // Live values for the native listeners (registered once).
  const refreshingRef = useRef(false);
  const onRefreshRef = useRef(onRefresh);
  useEffect(() => {
    onRefreshRef.current = onRefresh;
  });

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    let startY: number | null = null;
    let pulling = false;
    let lastPull = 0;

    const reset = () => {
      startY = null;
      pulling = false;
      lastPull = 0;
      setDragging(false);
    };

    const onTouchStart = (e: TouchEvent) => {
      if (refreshingRef.current) return;
      if (el.scrollTop > 0) return;
      startY = e.touches[0].clientY;
      pulling = false;
    };

    const onTouchMove = (e: TouchEvent) => {
      if (startY === null || refreshingRef.current) return;
      const dy = e.touches[0].clientY - startY;
      if (!pulling) {
        // Only claim the gesture when the list is at its top and the finger
        // moves down; otherwise hand it back to native scrolling.
        if (dy <= 0 || el.scrollTop > 0) {
          startY = null;
          return;
        }
        pulling = true;
        setDragging(true);
      }
      // Claimed: stop the browser's overscroll and drive the pull.
      e.preventDefault();
      lastPull = Math.max(0, Math.min(MAX_PULL_PX, dy * 0.5));
      setPull(lastPull);
    };

    const onTouchEnd = () => {
      if (startY === null) return;
      const release = lastPull;
      reset();
      if (release >= ARM_THRESHOLD_PX * 0.5 && !refreshingRef.current) {
        refreshingRef.current = true;
        setRefreshing(true);
        setPull(ARM_THRESHOLD_PX * 0.5);
        void Promise.resolve(onRefreshRef.current()).finally(() => {
          refreshingRef.current = false;
          setRefreshing(false);
          setPull(0);
        });
      } else {
        setPull(0);
      }
    };

    el.addEventListener('touchstart', onTouchStart, { passive: true });
    el.addEventListener('touchmove', onTouchMove, { passive: false });
    el.addEventListener('touchend', onTouchEnd, { passive: true });
    el.addEventListener('touchcancel', onTouchEnd, { passive: true });
    return () => {
      el.removeEventListener('touchstart', onTouchStart);
      el.removeEventListener('touchmove', onTouchMove);
      el.removeEventListener('touchend', onTouchEnd);
      el.removeEventListener('touchcancel', onTouchEnd);
    };
  }, []);

  const armed = pull >= ARM_THRESHOLD_PX * 0.5;
  const indicatorVisible = pull > 4 || refreshing;

  return (
    <div className="relative flex-1 min-h-0 flex flex-col">
      {/* Bare indicator: no button chrome, and it rides ABOVE the content it
          pushes down (negative offset) so it never sits on top of the first
          card while the list translates. */}
      {indicatorVisible && (
        <div
          className="absolute left-1/2 -translate-x-1/2 z-10 pointer-events-none flex items-center justify-center"
          style={{ top: Math.max(-30, pull - 46), height: 34 }}
        >
          {refreshing ? (
            <PenroseMarch size={30} />
          ) : (
            <ArrowDown
              size={20}
              weight="bold"
              className={armed ? 'text-accent' : 'text-textMuted'}
              style={{
                opacity: Math.min(1, pull / (ARM_THRESHOLD_PX * 0.4)),
                transform: `scale(${Math.min(1, 0.6 + pull / MAX_PULL_PX)}) rotate(${armed ? 180 : 0}deg)`,
                transition: 'transform 0.15s ease-out',
              }}
            />
          )}
        </div>
      )}
      <div
        ref={scrollRef}
        className={`flex-1 min-h-0 overflow-y-auto ${className ?? ''}`}
        style={{
          WebkitOverflowScrolling: 'touch',
          overscrollBehaviorY: 'contain',
          transform: pull ? `translateY(${pull}px)` : undefined,
          transition: dragging ? 'none' : 'transform 0.2s cubic-bezier(0.2, 0.8, 0.2, 1)',
        }}
      >
        {children}
      </div>
    </div>
  );
};
