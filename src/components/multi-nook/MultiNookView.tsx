import React, { useMemo, useState, useCallback, useRef, useEffect } from 'react';
import { makeKey } from '../../utils/providerKey';
import type { ProviderId } from '../../types/providers';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  DragEndEvent,
  DragStartEvent,
  pointerWithin,
  rectIntersection,
  CollisionDetection,
  useDroppable,
} from '@dnd-kit/core';
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  rectSortingStrategy,
} from '@dnd-kit/sortable';
import { ArrowUpFromLine } from 'lucide-react';
import { MultiNookCell } from './MultiNookCell';
import MultiNookToolbar from './MultiNookToolbar';
import { MultiNookTutorial } from './MultiNookTutorial';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useTutorialStore } from '../../stores/tutorialStore';
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import { Logger } from '../../utils/logger';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { useMultiNookSync } from './useMultiNookSync';

const DOCK_DROP_ID = 'dock-drop-zone';
const UNDOCK_DROP_ID = 'undock-drop-zone';

// Prefix for draggable docked pill IDs to distinguish from sortable grid cells
const DOCKED_PREFIX = 'docked::';

export const MultiNookView: React.FC = () => {
  // Subscribe to the two pieces of state actually rendered here; take the
  // actions without subscribing (they never change identity). A bare
  // `usemultiNookStore()` re-rendered this grid, and therefore every tile, on
  // any store mutation at all.
  const slots = usemultiNookStore((s) => s.slots);
  const maximizedSlotId = usemultiNookStore((s) => s.maximizedSlotId);
  const { reorderSlots, dockSlot, undockSlot, batchLoadMissingStreams, refreshSlotMetadata, setMaximizedSlot } =
    usemultiNookStore.getState();
  const visibleSlots = useMemo(() => slots.filter((s) => !s.isMinimized), [slots]);
  const minimizedSlots = useMemo(() => slots.filter((s) => s.isMinimized), [slots]);

  // A maximized tile only counts while it's still a visible slot — guards against
  // a stale id (e.g. the tile was docked/removed) painting an empty overlay.
  const isMaximizing = maximizedSlotId != null && visibleSlots.some((s) => s.id === maximizedSlotId);

  // Esc restores the grid while a tile is filling the space.
  useEffect(() => {
    if (!isMaximizing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setMaximizedSlot(null);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isMaximizing, setMaximizedSlot]);

  // Mount the global Co-Stream Sync Controller
  useMultiNookSync();

  // Keep every visible tile's chat connected in the background — not just the
  // focused one. This is what lets the moderator-log pane (and badge metadata)
  // cover all streams on screen and makes switching the focused chat instant.
  // The focused tile's chat is also held by the main ChatWidget; the store
  // ref-counts subscribers, so the overlap is harmless and changing focus never
  // drops a connection. Diff against a ref (rather than release-all/acquire-all)
  // so unchanged channels keep a steady ref count across renders.
  // Tracked by the store's own key space (bare login for Twitch, provider:channel
  // otherwise), because a Kick and a Twitch channel can share a name and must not
  // share a ref count. The bare channel and provider are kept alongside, since
  // acquire/release take those, not the composite key.
  const connectedChatRef = useRef<Map<string, { channel: string; provider: ProviderId }>>(new Map());
  useEffect(() => {
    const desired = new Map<
      string,
      { channel: string; provider: ProviderId; channelId: string | null }
    >();
    for (const s of visibleSlots) {
      const provider = s.provider ?? 'twitch';
      desired.set(makeKey(provider, s.channelLogin), {
        channel: s.channelLogin,
        provider,
        channelId: s.channelId ?? null,
      });
    }
    for (const [key, want] of desired) {
      if (!connectedChatRef.current.has(key)) {
        connectedChatRef.current.set(key, { channel: want.channel, provider: want.provider });
        void acquireChannel(want.channel, want.channelId, want.provider).catch((err) =>
          Logger.error('[MultiNook] background chat acquire failed:', err),
        );
      }
    }
    for (const [key, held] of Array.from(connectedChatRef.current)) {
      if (!desired.has(key)) {
        connectedChatRef.current.delete(key);
        void releaseChannel(held.channel, held.provider).catch((err) =>
          Logger.warn('[MultiNook] background chat release failed:', err),
        );
      }
    }
  }, [visibleSlots]);

  // Release every background connection when leaving MultiNook.
  useEffect(() => {
    const held = connectedChatRef.current;
    return () => {
      for (const entry of Array.from(held.values())) {
        void releaseChannel(entry.channel, entry.provider).catch(() => {});
      }
      held.clear();
    };
  }, []);

  // Track drag state: which type of item is being dragged
  const [dragSource, setDragSource] = useState<'visible' | 'docked' | 'tutorial' | null>(null);

  // Clean up tutorial state precisely when a real stream is added
  useEffect(() => {
    if (slots.length > 0) {
      useTutorialStore.getState().reset();
    }
  }, [slots.length]);

  // Batch loader for concurrent instantiation. Tiles flagged offline (loadError)
  // are skipped so a failed/unreachable stream doesn't keep the loader re-firing.
  useEffect(() => {
    if (slots.some(s => !s.streamUrl && !s.loadError)) {
      batchLoadMissingStreams();
    }
  }, [slots, batchLoadMissingStreams]);

  // Titles and categories change mid-stream, so poll them. Keyed on the tile set
  // rather than on `slots` itself: volume drags and focus changes mutate slots
  // constantly and would restart the interval each time.
  const loginKey = useMemo(
    () => slots.map((s) => makeKey(s.provider ?? 'twitch', s.channelLogin)).sort().join(','),
    [slots],
  );
  useEffect(() => {
    if (!loginKey) return;
    // Hold this off the mount frame. Opening a grid already starts up to 25 HLS
    // proxies at once, and the patch re-renders every tile that gains a title;
    // landing both together is what starves the main thread and stalls the MSE
    // appends. Also covers preset loads, which build slots from cached data with
    // no Twitch round-trip at all.
    const first = setTimeout(() => void refreshSlotMetadata(), 3_000);
    return () => clearTimeout(first);
  }, [loginKey, refreshSlotMetadata]);
  // Steady-state poll. Purely an on-screen surface, so it pauses while the
  // window is hidden rather than hitting Helix from the tray.
  useVisibleInterval(refreshSlotMetadata, 120_000);

  // Build a map of slot id -> visual order index for CSS-based reordering.
  const orderMap = useMemo(() => {
    const map = new Map<string, number>();
    visibleSlots.forEach((s, i) => map.set(s.id, i));
    return map;
  }, [visibleSlots]);

  // Stable DOM order: sort visible slots by id so DOM nodes never move on reorder.
  const stableDomSlots = useMemo(
    () => [...visibleSlots].sort((a, b) => a.id.localeCompare(b.id)),
    [visibleSlots]
  );

  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8,
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  // Custom collision detection: prioritize drop zones over grid cells
  const collisionDetection: CollisionDetection = useCallback((args) => {
    const pointerCollisions = pointerWithin(args);

    // Prioritize dock/undock zones
    const dockHit = pointerCollisions.find((c) => c.id === DOCK_DROP_ID);
    if (dockHit) return [dockHit];

    const undockHit = pointerCollisions.find((c) => c.id === UNDOCK_DROP_ID);
    if (undockHit) return [undockHit];

    // Fall back to standard grid collision for reordering
    return rectIntersection(args);
  }, []);

  const handleDragStart = (event: DragStartEvent) => {
    const id = event.active.id as string;
    if (id.startsWith('tutorial::') || id.startsWith('docked::tutorial::')) {
      setDragSource('tutorial');
    } else if (id.startsWith(DOCKED_PREFIX)) {
      setDragSource('docked');
    } else {
      setDragSource('visible');
    }
  };

  const handleDragEnd = (event: DragEndEvent) => {
    const currentSource = dragSource;
    setDragSource(null);
    const { active, over } = event;

    if (!over) return;

    const activeId = active.id as string;
    
    // Completely ignore tutorial drags; useDndMonitor internally handles them!
    if (activeId.startsWith('tutorial::') || activeId.startsWith('docked::tutorial::')) {
      return;
    }

    // Dragging a visible cell → dock zone = dock it
    if (over.id === DOCK_DROP_ID && currentSource === 'visible') {
      Logger.debug(`Docking slot ${activeId} via drag gesture`);
      dockSlot(activeId);
      return;
    }

    // Dragging a docked pill → undock zone = undock it
    if (over.id === UNDOCK_DROP_ID && currentSource === 'docked') {
      const realId = activeId.replace(DOCKED_PREFIX, '');
      Logger.debug(`Undocking slot ${realId} via drag gesture`);
      undockSlot(realId);
      return;
    }

    // Reorder (only for visible cells)
    if (currentSource === 'visible' && active.id !== over.id) {
      Logger.debug(`Reordering slot ${active.id} to ${over.id}`);
      const oldIndex = slots.findIndex((s) => s.id === active.id);
      const newIndex = slots.findIndex((s) => s.id === over.id);
      reorderSlots(arrayMove(slots, oldIndex, newIndex));
    }
  };

  const handleDragCancel = () => {
    setDragSource(null);
  };

  // ResizeObserver for Dynamic Flexbox Grid Math
  const containerRef = useRef<HTMLDivElement>(null);
  const [dimensions, setDimensions] = useState({ width: 0, height: 0 });

  useEffect(() => {
    if (!containerRef.current) return;
    const observer = new ResizeObserver((entries) => {
      // Use requestAnimationFrame to avoid "ResizeObserver loop limit exceeded" warning
      window.requestAnimationFrame(() => {
        if (!Array.isArray(entries) || !entries.length) return;
        setDimensions({
          width: entries[0].contentRect.width,
          height: entries[0].contentRect.height,
        });
      });
    });
    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [visibleSlots.length]); // re-bind if the entire component shifts dramatically

  // Flexbox Optimal Layout Engine
  const { cellWidth, cellHeight, optimalCols = 1 } = useMemo(() => {
    const len = visibleSlots.length;
    if (len === 0 || dimensions.width === 0 || dimensions.height === 0) {
      return { cellWidth: '100%', cellHeight: '100%' };
    }

    const W = dimensions.width;
    const H = dimensions.height;
    const gap = 8; // 8px (Tailwind gap-2)
    const ratio = 16 / 9;

    let bestArea = 0;
    let bestW = 0;
    let bestH = 0;
    let bestCols = 1;

    for (let cols = 1; cols <= len; cols++) {
      const rows = Math.ceil(len / cols);

      // Max width bounded
      const cellW_w = (W - (cols - 1) * gap) / cols;
      const cellH_w = cellW_w / ratio;

      // Max height bounded
      const cellH_h = (H - (rows - 1) * gap) / rows;
      const cellW_h = cellH_h * ratio;

      let cellW = cellW_w;
      let cellH = cellH_w;

      // If width-bounded height exceeds container height, we are constrained by height
      if (cellH * rows + (rows - 1) * gap > H) {
        cellW = cellW_h;
        cellH = cellH_h;
      }

      const area = cellW * cellH;
      if (area > bestArea) {
        bestArea = area;
        bestW = cellW;
        bestH = cellH;
        bestCols = cols;
      }
    }

    // Return as absolute floored pixels to completely avoid sub-pixel DOM wrapping bugs
    return {
      cellWidth: `${Math.floor(bestW)}px`,
      cellHeight: `${Math.floor(bestH)}px`,
      optimalCols: bestCols,
    };
  }, [visibleSlots.length, dimensions]);

  // Show dock zone only when dragging a visible cell
  const showDockZone = dragSource === 'visible' || dragSource === 'tutorial';
  // Show undock zone only when dragging a docked pill
  const showUndockZone = dragSource === 'docked';

  return (
    <div className="flex flex-col h-full bg-background relative overflow-hidden">
      <DndContext
        sensors={sensors}
        collisionDetection={collisionDetection}
        onDragStart={handleDragStart}
        onDragEnd={handleDragEnd}
        onDragCancel={handleDragCancel}
      >
        <MultiNookToolbar
          isDragging={showDockZone}
          dockDropId={DOCK_DROP_ID}
          dockedPrefix={DOCKED_PREFIX}
        />

        <div className={`flex-1 w-full relative overflow-hidden ${isMaximizing ? '' : 'p-2'}`} ref={containerRef}>
          {visibleSlots.length === 0 && !showUndockZone ? (
            <MultiNookTutorial />
          ) : (
            <div className="w-full h-full relative">
              {/* Undock drop zone — overlays the grid when dragging a docked pill */}
              {showUndockZone && (
                <UndockDropZone dropId={UNDOCK_DROP_ID} />
              )}

              <div className="w-full h-full flex flex-wrap justify-center content-center gap-2">
                <SortableContext
                  items={visibleSlots.map((s) => s.id)}
                  strategy={rectSortingStrategy}
                >
                  {stableDomSlots.map((slot) => {
                    const cssOrder = orderMap.get(slot.id) ?? 0;
                    const isThisMaximized = isMaximizing && slot.id === maximizedSlotId;

                    // While one tile fills the space, restyle it IN PLACE to an
                    // absolute full-bleed overlay (no remount → HLS keeps running,
                    // framer's `layout` animates the zoom) and collapse the rest to
                    // display:none. They stay mounted (still in the React tree), so
                    // their players keep buffering for an instant restore.
                    let cellStyle: React.CSSProperties = { width: cellWidth, height: cellHeight };
                    if (isMaximizing) {
                      cellStyle = isThisMaximized
                        ? { position: 'absolute', inset: 0, width: '100%', height: '100%', zIndex: 30, margin: 0 }
                        : { display: 'none' };
                    }

                    return (
                      <MultiNookCell
                        key={slot.id}
                        slot={slot}
                        cssOrder={cssOrder * 2}
                        customStyle={cellStyle}
                        isMaximized={isThisMaximized}
                      />
                    );
                  })}
                </SortableContext>
                
                {/* Odd leftover streams sit on top (an upward-pointing pyramid). */}
                {/* Flexbox naturally flows left-to-right, putting leftovers on the bottom. */}
                {/* We reverse the pyramid visually by inserting a flex break with an exact CSS order */}
                {/* that forces the first visual row to carry the deficit. */}
                {(visibleSlots.length % optimalCols) > 0 && (
                  <div 
                    style={{ 
                      flexBasis: '100%', 
                      height: 0, 
                      margin: 0, 
                      order: (visibleSlots.length % optimalCols) * 2 - 1 
                    }} 
                    aria-hidden="true" 
                  />
                )}
                
                {/* Keep minimized streams mounted in the DOM to avoid HLS cold-start buffering */}
                {minimizedSlots.map((slot) => (
                  <div key={slot.id} className="hidden">
                    <MultiNookCell slot={slot} />
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </DndContext>
    </div>
  );
};

/** Undock droppable overlay that appears over the grid when dragging a docked stream */
const UndockDropZone: React.FC<{ dropId: string }> = ({ dropId }) => {
  const { setNodeRef, isOver } = useDroppable({ id: dropId });

  return (
    <div
      ref={setNodeRef}
      className={`
        absolute inset-0 z-20 flex items-center justify-center rounded-xl pointer-events-auto
        transition-[background-color,border-color,transform] duration-300 ease-out backdrop-blur-sm
        ${isOver
          ? 'bg-transparent border-2 border-accent/60 shadow-[0_0_20px_rgba(var(--color-accent-rgb),0.1)_inset]'
          : 'bg-transparent border-2 border-dashed border-white/10'
        }
      `}
    >
      <div className={`flex items-center gap-3 transition-all duration-300 ${isOver ? 'scale-105' : ''}`}>
        <ArrowUpFromLine
          size={20}
          className={`transition-colors duration-300 ${isOver ? 'text-accent' : 'text-textMuted'}`}
        />
        <span className={`text-sm font-bold uppercase tracking-widest transition-colors duration-300 ${isOver ? 'text-accent' : 'text-textMuted'}`}>
          {isOver ? 'Release to restore' : 'Drop here to restore to grid'}
        </span>
      </div>
    </div>
  );
};

