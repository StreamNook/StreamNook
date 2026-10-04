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
  arraySwap,
  SortableContext,
  sortableKeyboardCoordinates,
  rectSortingStrategy,
  rectSwappingStrategy,
} from '@dnd-kit/sortable';
import { ArrowUpFromLine } from 'lucide-react';
import { MultiNookCell } from './MultiNookCell';
import MultiNookToolbar from './MultiNookToolbar';
import { MultiNookTutorial } from './MultiNookTutorial';
import { MultiNookTileMenu } from './MultiNookTileMenu';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useTutorialStore } from '../../stores/tutorialStore';
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import { Logger } from '../../utils/logger';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_MULTI_NOOK_LAYOUT, type MultiNookRaid, type MultiNookTileMeta } from '../../types';
import { useAppStore } from '../../stores/AppStore';
import { nookLayout } from './nookLayout';

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
  const { reorderSlots, dockSlot, undockSlot, batchLoadMissingStreams, setMaximizedSlot, makeMainSlot, reconcileTileCaps } =
    usemultiNookStore.getState();
  // Grid, or one main tile with the rest small (a setting Rust keeps).
  const layout = useAppStore((s) => s.settings.multi_nook_layout) ?? DEFAULT_MULTI_NOOK_LAYOUT;
  const mainLayout = layout.mode === 'main_row' || layout.mode === 'main_column';
  // The slider's live value while it is being dragged, else the saved one.
  const draftShare = usemultiNookStore((s) => s.draftShare);
  const stripShare = draftShare ?? layout.strip_share;
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
        void acquireChannel(want.channel, want.channelId, want.provider, { background: true }).catch((err) =>
          Logger.error('[MultiNook] background chat acquire failed:', err),
        );
      }
    }
    for (const [key, held] of Array.from(connectedChatRef.current)) {
      if (!desired.has(key)) {
        connectedChatRef.current.delete(key);
        void releaseChannel(held.channel, held.provider, { background: true }).catch((err) =>
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
        void releaseChannel(entry.channel, entry.provider, { background: true }).catch(() => {});
      }
      held.clear();
    };
  }, []);

  // Raids out of the grid's Twitch tiles. Rust holds one socket on their raid
  // topics (services/multi_nook_raids.rs); this only declares which channels,
  // keyed on the id set so a volume drag never re-sends it. Docked tiles are
  // included: the card is waiting when the tile comes back.
  const raidChannelKey = useMemo(
    () =>
      slots
        .filter((s) => (s.provider ?? 'twitch') === 'twitch' && s.channelId)
        .map((s) => s.channelId as string)
        .sort()
        .join(','),
    [slots],
  );
  useEffect(() => {
    invoke('set_multi_nook_raid_channels', {
      channelIds: raidChannelKey ? raidChannelKey.split(',') : [],
    }).catch((err) => Logger.warn('[MultiNook] raid channels update failed:', err));
  }, [raidChannelKey]);
  useEffect(() => {
    const unlisten = listen<MultiNookRaid>('multi-nook://raid', (e) => {
      usemultiNookStore.getState().markSlotsRaided(e.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
      // Leaving MultiNook closes the socket.
      invoke('set_multi_nook_raid_channels', { channelIds: [] }).catch(() => {});
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

  // Titles and categories change mid-stream. Rust polls them for every Twitch
  // tile in one batch and reports changes (services/multi_nook_meta). Keyed on
  // the channel set, so a volume drag never re-sends it.
  const metaLoginKey = useMemo(
    () =>
      Array.from(
        new Set(slots.filter((s) => (s.provider ?? 'twitch') === 'twitch').map((s) => s.channelLogin.toLowerCase())),
      )
        .sort()
        .join(','),
    [slots],
  );
  useEffect(() => {
    // Held off the opening frame: a grid starts up to 25 streams at once, and
    // titles landing on every tile in the same moment starves the main thread
    // and stalls playback. Also covers preset loads, built from cached data.
    const t = setTimeout(() => {
      invoke('set_multi_nook_meta_channels', { logins: metaLoginKey ? metaLoginKey.split(',') : [] }).catch((err) =>
        Logger.warn('[MultiNook] tile metadata channels update failed:', err),
      );
    }, 3_000);
    return () => clearTimeout(t);
  }, [metaLoginKey]);
  useEffect(() => {
    const unlisten = listen<MultiNookTileMeta[]>('multi-nook://meta', (e) => {
      usemultiNookStore.getState().applySlotMetadata(e.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
      // Leaving MultiNook ends the poll.
      invoke('set_multi_nook_meta_channels', { logins: [] }).catch(() => {});
    };
  }, []);

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

    // A main layout swaps two tiles rather than shifting the rest along, and a
    // swap into or out of the main spot makes the new tile main (sound and chat
    // follow, as with Make main).
    if (currentSource === 'visible' && active.id !== over.id && mainLayout) {
      const mainId = visibleSlots[0]?.id;
      const overId = over.id as string;
      if (activeId === mainId) {
        makeMainSlot(overId);
      } else if (overId === mainId) {
        makeMainSlot(activeId);
      } else {
        const a = slots.findIndex((s) => s.id === activeId);
        const b = slots.findIndex((s) => s.id === overId);
        if (a >= 0 && b >= 0) reorderSlots(arraySwap(slots, a, b));
      }
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

  // Where each visible tile goes for the stage's size (components/multi-nook/
  // nookLayout.ts). One engine for Grid and the main layouts, so switching is
  // a restyle and never moves a node.
  const placeById = useMemo(() => {
    const places = nookLayout(dimensions.width, dimensions.height, visibleSlots.length, layout.mode, stripShare);
    return new Map(visibleSlots.map((s, i) => [s.id, places[i]]));
  }, [visibleSlots, dimensions, layout.mode, stripShare]);

  // With a small-tile quality cap, a tile that changes role (made main, moved
  // to the strip, spotlighted, docked) restarts at its new size's quality.
  // Without one this finds nothing to do.
  const roleKey = `${layout.mode}|${layout.small_quality_cap ?? ''}|${maximizedSlotId ?? ''}|${visibleSlots.map((s) => s.id).join(',')}`;
  useEffect(() => {
    void reconcileTileCaps();
  }, [roleKey, reconcileTileCaps]);

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

              <div className="relative w-full h-full">
                <SortableContext
                  items={visibleSlots.map((s) => s.id)}
                  strategy={mainLayout ? rectSwappingStrategy : rectSortingStrategy}
                >
                  {stableDomSlots.map((slot) => {
                    const isThisMaximized = isMaximizing && slot.id === maximizedSlotId;
                    const place = placeById.get(slot.id);

                    // Each tile sits at its rectangle. While one tile fills the
                    // space it is restyled IN PLACE to a full-bleed overlay (no
                    // remount, so HLS keeps running and framer's `layout`
                    // animates the zoom) and the rest collapse to display:none,
                    // still mounted, so their players keep buffering.
                    let cellStyle: React.CSSProperties = place
                      ? { position: 'absolute', left: place.x, top: place.y, width: place.w, height: place.h, margin: 0 }
                      : { display: 'none' };
                    if (isMaximizing) {
                      cellStyle = isThisMaximized
                        ? { position: 'absolute', inset: 0, width: '100%', height: '100%', zIndex: 30, margin: 0 }
                        : { display: 'none' };
                    }

                    return (
                      <MultiNookCell
                        key={slot.id}
                        slot={slot}
                        customStyle={cellStyle}
                        isMaximized={isThisMaximized}
                        sizeTier={isThisMaximized ? 'full' : (place?.tier ?? 'full')}
                        canMakeMain={mainLayout && !isMaximizing && !!place && !place.main}
                      />
                    );
                  })}
                </SortableContext>

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

      {/* Tile right-click menu. Mounted beside the grid rather than inside a
          cell so it is not clipped by a tile's `overflow-hidden`, and so it
          survives the cell unmounting under it (closing a stream from its own
          menu is the ordinary case). */}
      <MultiNookTileMenu />
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

