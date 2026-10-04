import React, { useMemo } from 'react';
import {
  SortableContext,
  rectSortingStrategy,
  useSortable,
  arrayMove,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useDndMonitor, DragOverlay, useDroppable } from '@dnd-kit/core';
import { LayoutGrid, Plus, Volume2, Move, Minimize2, GripHorizontal, ArrowUpFromLine, MessageSquare, RefreshCcw } from 'lucide-react';
import { useTutorialStore } from '../../stores/tutorialStore';
import { Tooltip } from '../ui/Tooltip';
import { motion, AnimatePresence } from 'framer-motion';

const DOCK_DROP_ID = 'dock-drop-zone';

interface TutorialCardData {
  id: string;
  icon: React.ReactNode;
  title: string;
  desc: React.ReactNode;
  colorClass: string;
}

const TUTORIAL_DATA: Record<string, TutorialCardData> = {
  'tutorial::add': {
    id: 'tutorial::add',
    icon: <Plus className="w-5 h-5 text-accent" />,
    title: 'Add Streams',
    desc: (
      <span>
        Use the <strong className="text-textPrimary">+</strong> button top-right, right-click any stream, or click the <strong className="text-textPrimary">top right arrow</strong> on any stream card to add it to your grid.
      </span>
    ),
    colorClass: 'text-accent',
  },
  'tutorial::focus': {
    id: 'tutorial::focus',
    icon: <Volume2 className="w-5 h-5 text-blue-400" />,
    title: 'Smart Audio Focus',
    desc: (
      <span>
        Click the body of any card to <strong>Focus</strong> its audio (indicated by a gentle glow). Click again to unfocus and instantly hear all streams simultaneously.
      </span>
    ),
    colorClass: 'text-blue-400',
  },
  'tutorial::drag': {
    id: 'tutorial::drag',
    icon: <Move className="w-5 h-5 text-purple-400" />,
    title: 'Drag & Drop Layout',
    desc: 'Grab the top-center handle of any card and drag it around to instantly rearrange your entire grid layout.',
    colorClass: 'text-purple-400',
  },
  'tutorial::dock': {
    id: 'tutorial::dock',
    icon: <Minimize2 className="w-5 h-5 text-emerald-400" />,
    title: 'Docking & Background',
    desc: 'For this tutorial, drag this specific card up to the top toolbar to dock it. (In real grids, any stream can be docked to hide it!)',
    colorClass: 'text-emerald-400',
  },
  'tutorial::chat': {
    id: 'tutorial::chat',
    icon: <MessageSquare className="w-5 h-5 text-pink-400" />,
    title: 'Chat Control',
    desc: 'The focused stream automatically commands the chat view, but you can switch it manually to any stream, or hide chat entirely for a cinematic layout.',
    colorClass: 'text-pink-400',
  },
  'tutorial::sync': {
    id: 'tutorial::sync',
    icon: <RefreshCcw className="w-5 h-5 text-orange-400" />,
    title: 'Stream Resync',
    desc: 'Click the circular arrows in the toolbar to line every stream up at the same delay behind live. They stay together until you close MultiNook.',
    colorClass: 'text-orange-400',
  },
};

const TutorialUndockDropZone: React.FC<{ dropId: string }> = ({ dropId }) => {
  const { setNodeRef, isOver } = useDroppable({ id: dropId });

  return (
    <div
      ref={setNodeRef}
      className={`
        absolute inset-0 z-20 flex items-center justify-center rounded-xl pointer-events-auto
        transition-all duration-300 ease-out backdrop-blur-sm
        ${isOver
          ? 'bg-transparent border-2 border-emerald-400/60 shadow-[0_0_20px_color-mix(in_srgb,var(--color-success)_10%,transparent)_inset]'
          : 'bg-transparent border-2 border-dashed border-white/10'
        }
      `}
    >
      <div className={`flex items-center gap-3 transition-all duration-300 ${isOver ? 'scale-105' : ''}`}>
        <ArrowUpFromLine
          size={20}
          className={`transition-colors duration-300 ${isOver ? 'text-emerald-400' : 'text-textMuted'}`}
        />
        <span className={`text-sm font-bold uppercase tracking-widest transition-colors duration-300 ${isOver ? 'text-emerald-400' : 'text-textMuted'}`}>
          {isOver ? 'Release to restore' : 'Drop here to restore to grid'}
        </span>
      </div>
    </div>
  );
};

const HelpCardVisual = React.forwardRef<
  HTMLDivElement,
  {
    data: TutorialCardData;
    isFocused: boolean;
    isDockCard: boolean;
    isDraggingGhost?: boolean;
    isOverlay?: boolean;
    style?: React.CSSProperties;
    onClick?: (e: React.MouseEvent) => void;
    dragAttributes?: any;
    dragListeners?: any;
  }
>(({ data, isFocused, isDockCard, isDraggingGhost, isOverlay, style, onClick, dragAttributes, dragListeners }, ref) => (
  <div
    ref={ref}
    style={style}
    onClick={onClick}
    className={`
      flex flex-col gap-2 p-5 rounded-xl bg-glass/10 border border-white/5 
      transition-all duration-300 relative group select-none cursor-pointer
      ${isDraggingGhost ? 'opacity-40 scale-95 border-dashed' : ''}
      ${isOverlay ? 'opacity-100 scale-105 shadow-2xl rotate-2 ring-1 ring-white/20 z-50 bg-surface/90 backdrop-blur-xl' : 'hover:bg-glass/20'}
      ${isFocused && !isDraggingGhost && !isOverlay ? 'shadow-[0_0_20px_color-mix(in_srgb,var(--color-info)_30%,transparent)]' : ''}
    `}
  >
    {!isDraggingGhost && (
      <div className="absolute inset-0 bg-accent/5 opacity-0 group-hover:opacity-100 transition-opacity duration-500 pointer-events-none rounded-xl" />
    )}
    
    {/* Always render the uniform grab handle, matching MultiNookCell */}
    <div className="absolute left-1/2 -translate-x-1/2 top-1.5 z-20">
      <Tooltip content="Drag to rearrange" delay={500} side="top">
        <div 
          onClick={(e) => e.stopPropagation()}
          className={`cursor-grab active:cursor-grabbing px-3 py-1 rounded-md transition-colors [&_*]:cursor-grab flex items-center justify-center ${
            isDockCard ? 'hover:bg-emerald-400/10 text-emerald-400/50' : 'hover:bg-white/10 text-white/50'
          }`}
          {...dragAttributes}
          {...dragListeners}
        >
          <GripHorizontal className={`w-5 h-5 drop-shadow-md transition-colors ${
            isDockCard ? 'group-hover:text-emerald-400/80' : 'group-hover:text-white/80'
          }`} />
        </div>
      </Tooltip>
    </div>

    <div className="flex items-center gap-3 mb-1 relative z-10 pt-4">
      <div className="p-2 rounded-lg bg-white/5">
        {data.icon}
      </div>
      <h3 className="font-semibold text-textPrimary">{data.title}</h3>
    </div>
    <p className="text-sm text-textMuted leading-relaxed relative z-10 pointer-events-none">
      {data.desc}
    </p>
  </div>
));
HelpCardVisual.displayName = 'HelpCardVisual';

const InteractiveHelpCard: React.FC<{ data: TutorialCardData }> = ({ data }) => {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: data.id,
  });

  const { focusedId, setFocusedId } = useTutorialStore();
  const isFocused = focusedId === data.id;
  const isDockCard = data.id === 'tutorial::dock';

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    zIndex: isDragging ? 50 : 1,
  };

  const handleClick = () => {
    // Only focus if it's not a drag action
    if (!isDragging) {
      if (isFocused) {
        setFocusedId(null);
      } else {
        setFocusedId(data.id);
      }
    }
  };

  return (
    <HelpCardVisual
      ref={setNodeRef}
      data={data}
      isFocused={isFocused}
      isDockCard={isDockCard}
      isDraggingGhost={isDragging}
      style={style}
      onClick={handleClick}
      dragAttributes={attributes}
      dragListeners={listeners}
    />
  );
};

export const MultiNookTutorial: React.FC = () => {
  const { order, setOrder, isDocked, setIsDocked, focusedId } = useTutorialStore();
  const [activeDragId, setActiveDragId] = React.useState<string | null>(null);
  const [isGridVisible, setIsGridVisible] = React.useState(false);

  // Filter out the dock card if it's currently docked
  const visibleCardIds = useMemo(() => {
    return order.filter((id) => {
      if (id === 'tutorial::dock' && isDocked) return false;
      return true;
    });
  }, [order, isDocked]);

  // Hook into the parent DndContext directly
  useDndMonitor({
    onDragStart: (event) => {
      const activeId = String(event.active.id);
      if (activeId.startsWith('tutorial::') || activeId.startsWith('docked::tutorial::')) {
        setActiveDragId(activeId);
      }
    },
    onDragEnd: (event) => {
      setActiveDragId(null);
      const { active, over } = event;
      if (!over) return;
      const activeId = String(active.id);

      // We ONLY care about tutorial events
      if (!activeId.startsWith('tutorial::') && !activeId.startsWith('docked::tutorial::')) return;

      // 1. Docking the mock "Docking" card
      if (activeId === 'tutorial::dock' && over.id === DOCK_DROP_ID) {
        setIsDocked(true);
        return;
      }

      // 2. Undocking the mock "Docking" card
      // (The real MultiNookView undock zone uses 'undock-drop-zone')
      if (activeId === 'docked::tutorial::dock' && over.id === 'undock-drop-zone') {
        setIsDocked(false);
        return;
      }

      // 3. Reordering standard tutorial cards
      if (activeId.startsWith('tutorial::') && over.id.toString().startsWith('tutorial::') && active.id !== over.id) {
        const oldIndex = order.indexOf(activeId);
        const newIndex = order.indexOf(String(over.id));
        if (oldIndex !== -1 && newIndex !== -1) {
          setOrder(arrayMove(order, oldIndex, newIndex));
        }
      }
    },
    onDragCancel: () => {
      setActiveDragId(null);
    }
  });

  const isUndocking = activeDragId === 'docked::tutorial::dock';

  return (
    <div className="w-full h-full flex flex-col items-center justify-center text-textMuted border-2 border-dashed border-borderSubtle bg-glass/20 rounded-xl p-6 overflow-y-auto custom-scrollbar relative">
      <div className={`flex flex-col items-center max-w-3xl w-full animate-in fade-in slide-in-from-bottom-4 duration-500 ${isUndocking ? 'opacity-0' : 'opacity-100'}`}>
        <button 
          onClick={() => setIsGridVisible(prev => !prev)}
          className={`
            w-16 h-16 flex items-center justify-center mb-6 glass-input
            transition-all duration-300 cursor-pointer group
            ${isGridVisible ? '!bg-surface-active !border-accent/30 !shadow-[inset_5px_5px_12px_-3px_rgba(0,0,0,0.6),inset_-5px_-5px_12px_-3px_rgba(255,255,255,0.15),0_0_20px_rgba(var(--color-accent-rgb),0.1)]' : ''}
          `}
          style={{ borderRadius: '1rem' }}
        >
          <LayoutGrid className={`w-8 h-8 transition-all duration-300 ${!isGridVisible ? 'text-accent group-hover:drop-shadow-[0_0_10px_rgba(var(--color-accent-rgb),0.6)]' : 'text-accent drop-shadow-[0_0_8px_rgba(var(--color-accent-rgb),0.5)]'}`} />
        </button>
        
        <h2 className="text-2xl font-bold text-textPrimary mb-2 tracking-tight">Welcome to MultiNook</h2>
        <p className="text-textMuted text-center max-w-md mb-8 transition-opacity duration-300 leading-relaxed">
          Your ultimate interactive streaming canvas. Hit the + button top-right to start adding streams, or {
            !isGridVisible 
              ? <span>click the <strong className="italic text-textPrimary">icon</strong> above to learn the ropes.</span> 
              : <span>click the active <strong className="italic text-textPrimary">icon</strong> to pack up the sandbox.</span>
          }
        </p>

        <AnimatePresence mode="wait">
          {isGridVisible && (
            <motion.div 
              key="tutorial-grid"
              initial={{ opacity: 0, y: 30 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 20 }}
              transition={{ duration: 0.4, ease: 'easeOut' }}
              className="w-full flex flex-col items-center"
            >
              <div className="flex items-center justify-start w-full max-w-[900px] mb-4">
                <span className="text-xs font-bold text-textSecondary uppercase tracking-widest pl-2">Tutorial Sandbox</span>
              </div>
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 w-full">
                <SortableContext items={visibleCardIds} strategy={rectSortingStrategy}>
                  {visibleCardIds.map((id) => (
                    <InteractiveHelpCard key={id} data={TUTORIAL_DATA[id]} />
                  ))}
                </SortableContext>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {isUndocking && <TutorialUndockDropZone dropId="undock-drop-zone" />}

      {/* DragOverlay mounted here using position fixed will automatically escape the overflow boundaries of the parent */}
      <DragOverlay zIndex={9999} dropAnimation={null}>
        {activeDragId && activeDragId.startsWith('tutorial::') && TUTORIAL_DATA[activeDragId] ? (
          <HelpCardVisual
            data={TUTORIAL_DATA[activeDragId]}
            isFocused={focusedId === activeDragId}
            isDockCard={activeDragId === 'tutorial::dock'}
            isOverlay={true}
          />
        ) : null}
      </DragOverlay>
    </div>
  );
};
