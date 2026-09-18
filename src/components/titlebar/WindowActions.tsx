import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { LayoutGrid, MessagesSquare, Undo2 } from 'lucide-react';

import { useAppStore } from '../../stores/AppStore';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { Logger } from '../../utils/logger';
import { Tooltip } from '../ui/Tooltip';

/**
 * The two window actions that live in the title bar.
 *
 * They used to sit in Home's tab strip, which put a window action inside page
 * navigation and made both unreachable from every other view. MultiNook tiles
 * players and MultiChat spawns a popout: neither is a page, and both act on the
 * window, so they belong with drops, plugins and settings.
 *
 * Icons rather than labels. Everything else in that bar is an icon, and the
 * words cost about 170px — enough to push the left cluster into the Dynamic
 * Island, which owns dead centre and expands over whatever is beside it.
 */

const FlyingDot = ({ startX, startY, targetX, targetY }: { startX: number, startY: number, targetX: number, targetY: number }) => {
    const [isFlying, setIsFlying] = useState(false);

    useEffect(() => {
        // Two frames, not a timer. A transition only runs if the browser has
        // actually PAINTED the start state first, and a 10ms timeout can fire
        // before that paint on a busy frame -- which is exactly the frame this
        // fires on, right after a card click and a store update. When it loses
        // that race the element is simply born at its destination and there is
        // no travel at all, which is half of what the stutter was.
        let second = 0;
        const first = requestAnimationFrame(() => {
            second = requestAnimationFrame(() => setIsFlying(true));
        });
        return () => {
            cancelAnimationFrame(first);
            cancelAnimationFrame(second);
        };
    }, []);

    return (
        <div 
            className="fixed z-[9999] pointer-events-none flex h-5 w-5 items-center justify-center rounded-full bg-red-500 shadow-[0_0_15px_color-mix(in_srgb,var(--color-live)_80%,transparent)]"
            // Moved with `transform`, not `left`/`top`.
            //
            // left/top are LAYOUT properties: the browser has to run layout and
            // paint for every frame of the flight, on the main thread, competing
            // with the React render and the store update that just fired. A
            // transform is handled by the compositor and touches neither.
            //
            // `transition: all` made it worse again by putting every animatable
            // property on the clock, box-shadow included, and this element
            // carries a 15px glow. Naming the two that actually change means the
            // shadow is rasterized once instead of every frame.
            style={{
                left: startX,
                top: startY,
                transform: isFlying
                    ? `translate3d(${targetX - startX}px, ${targetY - startY}px, 0) scale(0.3)`
                    : 'translate3d(0, 0, 0) scale(1)',
                opacity: isFlying ? 0.3 : 1,
                transition:
                    'transform 500ms cubic-bezier(0.25, 1, 0.5, 1),' +
                    ' opacity 500ms cubic-bezier(0.25, 1, 0.5, 1)',
                willChange: 'transform, opacity',
            }}
        >
           <LayoutGrid size={12} className="text-white" />
        </div>
    );
};

const ReverseFlyingDot = ({ startX, startY, targetX, targetY }: { startX: number, startY: number, targetX: number, targetY: number }) => {
    const [isFlying, setIsFlying] = useState(false);

    useEffect(() => {
        // Two frames, not a timer. A transition only runs if the browser has
        // actually PAINTED the start state first, and a 10ms timeout can fire
        // before that paint on a busy frame -- which is exactly the frame this
        // fires on, right after a card click and a store update. When it loses
        // that race the element is simply born at its destination and there is
        // no travel at all, which is half of what the stutter was.
        let second = 0;
        const first = requestAnimationFrame(() => {
            second = requestAnimationFrame(() => setIsFlying(true));
        });
        return () => {
            cancelAnimationFrame(first);
            cancelAnimationFrame(second);
        };
    }, []);

    return (
        <div 
            className="fixed z-[9999] pointer-events-none flex h-5 w-5 items-center justify-center rounded-full bg-accent shadow-[0_0_15px_rgba(var(--color-accent-rgb),0.8)]"
            // Moved with `transform`, not `left`/`top`.
            //
            // left/top are LAYOUT properties: the browser has to run layout and
            // paint for every frame of the flight, on the main thread, competing
            // with the React render and the store update that just fired. A
            // transform is handled by the compositor and touches neither.
            //
            // `transition: all` made it worse again by putting every animatable
            // property on the clock, box-shadow included, and this element
            // carries a 15px glow. Naming the two that actually change means the
            // shadow is rasterized once instead of every frame.
            style={{
                left: startX,
                top: startY,
                transform: isFlying
                    ? `translate3d(${targetX - startX}px, ${targetY - startY}px, 0) scale(1)`
                    : 'translate3d(0, 0, 0) scale(0.3)',
                opacity: isFlying ? 1 : 0.3,
                transition:
                    'transform 500ms cubic-bezier(0.25, 1, 0.5, 1),' +
                    ' opacity 500ms cubic-bezier(0.25, 1, 0.5, 1)',
                willChange: 'transform, opacity',
            }}
        >
           <Undo2 size={10} className="text-white" />
        </div>
    );
};

export const MultiNookToggle = () => {
    const { isMultiNookActive, toggleMultiNook, slots, flyingAnimation, recallAnimation } = usemultiNookStore();
    // Action only, stable for the store's lifetime: no subscription needed.
    const { toggleHome } = useAppStore.getState();
    const [animateBadge, setAnimateBadge] = useState(false);
    const buttonRef = useRef<HTMLButtonElement>(null);
    const [flyingDots, setFlyingDots] = useState<Array<{ id: number, startX: number, startY: number, targetX: number, targetY: number }>>([]);
    const [reverseDots, setReverseDots] = useState<Array<{ id: number, startX: number, startY: number, targetX: number, targetY: number }>>([]);

    useEffect(() => {
        if (flyingAnimation && buttonRef.current) {
            const rect = buttonRef.current.getBoundingClientRect();
            // Target is the top right of the button where the badge will sit
            const targetX = rect.right - 10;
            const targetY = rect.top - 5;
            
            const newDot = {
                id: flyingAnimation.id,
                startX: flyingAnimation.x,
                startY: flyingAnimation.y,
                targetX,
                targetY
            };
            
            setFlyingDots(prev => [...prev, newDot]);
            
            setTimeout(() => {
                setFlyingDots(prev => prev.filter(d => d.id !== newDot.id));
                setAnimateBadge(true);
                setTimeout(() => setAnimateBadge(false), 200);
            }, 500); 
        }
    }, [flyingAnimation]);

    // Handle reverse recall animation — dot flies from badge to card
    useEffect(() => {
        if (recallAnimation) {
            const newDot = {
                id: recallAnimation.id,
                startX: recallAnimation.sourceX,
                startY: recallAnimation.sourceY,
                targetX: recallAnimation.targetX,
                targetY: recallAnimation.targetY,
            };
            
            queueMicrotask(() => setReverseDots(prev => [...prev, newDot]));
            
            setTimeout(() => {
                setReverseDots(prev => prev.filter(d => d.id !== newDot.id));
            }, 600);
        }
    }, [recallAnimation]);

    return (
        <>
            <Tooltip content={isMultiNookActive ? 'Return to MultiNook' : 'Enter MultiNook'} side="bottom">
                <button
                    id="multinook-return-button"
                    ref={buttonRef}
                    onClick={() => {
                        if (isMultiNookActive) {
                            toggleHome();
                        } else {
                            toggleMultiNook();
                        }
                    }}
                    data-tauri-drag-region="false"
                    className={`titlebar-icon-btn relative ${isMultiNookActive ? 'is-active' : ''}`}
                >
                    <LayoutGrid size={17} />
                    {!isMultiNookActive && slots.length > 0 && (
                        <span 
                            className={`absolute -top-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full glass-button !bg-error/30 text-[9px] font-extrabold text-white !shadow-[0_2px_10px_color-mix(in_srgb,var(--color-error)_40%,transparent),inset_0_1px_rgba(255,255,255,0.2)] z-50 ${
                                animateBadge ? 'scale-150 ring-2 ring-error transition-transform duration-200' : 'scale-100 transition-transform duration-500'
                            }`}
                        >
                            {slots.length}
                        </span>
                    )}
                </button>
            </Tooltip>
            {flyingDots.length > 0 && typeof document !== 'undefined' && createPortal(
                flyingDots.map(dot => (
                    <FlyingDot key={dot.id} {...dot} />
                )),
                document.body
            )}
            {reverseDots.length > 0 && typeof document !== 'undefined' && createPortal(
                reverseDots.map(dot => (
                    <ReverseFlyingDot key={dot.id} {...dot} />
                )),
                document.body
            )}
        </>
    );
};

// Top-nav entry point for the MultiChat popout, sitting beside MultiNook so the
// two "multi" workspaces live together. Clicking opens the popout window — or
// focuses it if one's already open — via openMultiChatWindow with no channel
// (which seeds an empty window, or restores its previous tabs). The count badge
// mirrors MultiNook's: it shows how many channels are currently popped out,
// which main tracks through the `channelsInPopouts` aggregate the popout
// broadcasts. No outer glow on the active state (no-glow aesthetic); the badge
// uses the approved soft-shadow notification recipe.
export const MultiChatButton = () => {
    const channelsInPopouts = useAppStore((s) => s.channelsInPopouts);
    const count = channelsInPopouts.size;
    const active = count > 0;

    const handleOpen = async () => {
        try {
            const { openMultiChatWindow } = await import('../../utils/multichatWindow');
            await openMultiChatWindow({});
        } catch (err) {
            Logger.error('[TitleBar] openMultiChatWindow failed:', err);
        }
    };

    return (
        <Tooltip content="Open MultiChat" side="bottom">
            <button
                onClick={handleOpen}
                data-tauri-drag-region="false"
                className={`titlebar-icon-btn relative ${active ? 'is-active' : ''}`}
            >
                <MessagesSquare size={17} />
                {count > 0 && (
                    <span className="absolute -top-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full glass-button !bg-accent/30 text-[9px] font-extrabold text-white !shadow-[0_2px_10px_rgba(var(--color-accent-rgb),0.35),inset_0_1px_rgba(255,255,255,0.2)] z-50">
                        {count}
                    </span>
                )}
            </button>
        </Tooltip>
    );
};

