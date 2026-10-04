import { useTooltipStore } from "../../stores/TooltipStore";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { useState, useCallback } from "react";

// Default tooltip container chrome. Callers can override via the
// containerClassName prop on <Tooltip>; when set, this default is replaced
// entirely so e.g. StreamNookBadge can render a pill-shaped popover.
const DEFAULT_TOOLTIP_CONTAINER_CLASS =
  "rounded-md bg-glass-ink/80 px-2.5 py-1.5 text-xs font-medium text-textPrimary shadow-xl backdrop-blur-xl border border-white/10 max-w-xs break-words pointer-events-none text-center leading-tight";

export const TooltipManager = () => {
  const { isVisible, content, rect, side: initialSide, containerClassName } = useTooltipStore();
  const [dimensions, setDimensions] = useState({ width: 0, height: 0, content: null as any });

  // Reset dimensions during render if content changes (React 18 pattern)
  if (isVisible && content !== dimensions.content) {
    setDimensions({ width: 0, height: 0, content });
  }

  const measureRef = useCallback((node: HTMLDivElement | null) => {
    if (node !== null) {
      const { offsetWidth, offsetHeight } = node;
      if (offsetWidth !== dimensions.width || offsetHeight !== dimensions.height) {
        setDimensions(prev => ({ ...prev, width: offsetWidth, height: offsetHeight }));
      }
    }
  }, [dimensions.width, dimensions.height]);

  if (typeof document === "undefined") {
    return null;
  }

  // Where the tooltip sits (anchor point plus a static percentage translate),
  // and the few pixels it drifts in from, toward the element. The drift is a
  // fixed distance, not a share of the tooltip's height: a tall emote preview
  // must not start out underneath the emote.
  let x = 0;
  let y = 0;
  let animateX = "-50%";
  let animateY = "-50%";
  let driftX = 0;
  let driftY = 0;

  if (rect) {
    const GAP = 8; // distance from rect
    const DRIFT = 6; // px the tooltip moves while it fades in
    let currentSide = initialSide;

    // Apply baseline unconstrained constraints
    const applyTop = () => {
      x = rect.left + rect.width / 2;
      y = rect.top - GAP;
      animateX = "-50%";
      animateY = "-100%";
      driftX = 0;
      driftY = DRIFT;
    };
    const applyBottom = () => {
      x = rect.left + rect.width / 2;
      y = rect.bottom + GAP;
      animateX = "-50%";
      animateY = "0%";
      driftX = 0;
      driftY = -DRIFT;
    };
    const applyLeft = () => {
      x = rect.left - GAP;
      y = rect.top + rect.height / 2;
      animateX = "-100%";
      animateY = "-50%";
      driftX = DRIFT;
      driftY = 0;
    };
    const applyRight = () => {
      x = rect.right + GAP;
      y = rect.top + rect.height / 2;
      animateX = "0%";
      animateY = "-50%";
      driftX = -DRIFT;
      driftY = 0;
    };

    switch (currentSide) {
      case "top": applyTop(); break;
      case "bottom": applyBottom(); break;
      case "left": applyLeft(); break;
      case "right": applyRight(); break;
    }

    if (dimensions.width > 0 && dimensions.height > 0) {
      const padding = 12;
      const windowWidth = window.innerWidth;
      const windowHeight = window.innerHeight;

      // 1. Flip axis if overflowing viewport boundaries
      if (currentSide === "top" && y - dimensions.height < padding) {
        applyBottom();
        currentSide = "bottom";
      } else if (currentSide === "bottom" && y + dimensions.height > windowHeight - padding) {
        applyTop();
        currentSide = "top";
      } else if (currentSide === "left" && x - dimensions.width < padding) {
        applyRight();
        currentSide = "right";
      } else if (currentSide === "right" && x + dimensions.width > windowWidth - padding) {
        applyLeft();
        currentSide = "left";
      }

      // 2. Clamp orthogonal axis (e.g. horizontal clamp for top/bottom tooltips)
      if (currentSide === "top" || currentSide === "bottom") {
        const leftEdge = x - dimensions.width / 2;
        const rightEdge = x + dimensions.width / 2;
        
        if (leftEdge < padding) {
          x += (padding - leftEdge);
        } else if (rightEdge > windowWidth - padding) {
          x -= (rightEdge - (windowWidth - padding));
        }
      } else { // left or right
        const topEdge = y - dimensions.height / 2;
        const bottomEdge = y + dimensions.height / 2;
        
        if (topEdge < padding) {
          y += (padding - topEdge);
        } else if (bottomEdge > windowHeight - padding) {
          y -= (bottomEdge - (windowHeight - padding));
        }
      }
    }
  }

  return createPortal(
    <div
      className="pointer-events-none fixed inset-0 z-[99999]"
      style={{ isolation: "isolate" }}
    >
      <AnimatePresence>
        {isVisible && content && rect && (
          // The outer node holds the final placement; only the inner one
          // animates, so the motion is a short drift, never a slide across
          // the tooltip's own height.
          <motion.div
            key="tooltip"
            style={{
              position: "absolute",
              left: `${x}px`,
              top: `${y}px`,
              x: animateX,
              y: animateY,
            }}
            className="pointer-events-none"
          >
          <motion.div
            ref={measureRef}
            initial={{ opacity: 0, scale: 0.97, x: driftX, y: driftY }}
            animate={{
              opacity: 1,
              scale: 1,
              x: 0,
              y: 0,
              transition: { duration: 0.09, ease: "easeOut" },
            }}
            exit={{
              opacity: 0,
              scale: 0.97,
              transition: { duration: 0.06, ease: "easeIn" },
            }}
            className="pointer-events-none"
          >
            {/* The chrome (including backdrop-blur) sits on a STATIC child, not
                on the animated node above. A backdrop-filter on the element that
                is being transformed keeps the animation off the compositor and
                forces a per-frame re-raster — and this fires on every hover over
                any emote, badge or username in the scrolling chat list. Same
                split as MajorCologneChrome.css. */}
            <div
              // Frosted glass styling by default; callers can override via the
              // containerClassName prop on <Tooltip> for a different shape/skin.
              className={containerClassName ?? DEFAULT_TOOLTIP_CONTAINER_CLASS}
            >
              {content}
            </div>
          </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>,
    document.body
  );
};
