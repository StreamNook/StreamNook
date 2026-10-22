import React, { useEffect, useId } from "react";
import { useTooltipStore } from "../../stores/TooltipStore";
import { IS_MOBILE } from "../../utils/platform";

/** The props the tooltip reads from, and injects into, its single child. */
export interface TooltipChildProps {
  onMouseEnter?: (e: React.MouseEvent) => void;
  onMouseLeave?: (e: React.MouseEvent) => void;
  onMouseMove?: (e: React.MouseEvent) => void;
  onPointerEnter?: (e: React.PointerEvent) => void;
  onPointerMove?: (e: React.PointerEvent) => void;
  onFocus?: (e: React.FocusEvent) => void;
  onBlur?: (e: React.FocusEvent) => void;
  disabled?: boolean;
  "aria-label"?: string;
  "aria-description"?: string;
  title?: string;
}

export interface TooltipProps {
  content: React.ReactNode | string;
  side?: "top" | "bottom" | "left" | "right";
  delay?: number;
  children: React.ReactElement<TooltipChildProps>;
  disabled?: boolean;
  // Optional override for the tooltip container's class list. When provided,
  // replaces the default chrome entirely (rounded-md, bg-black/80, border…).
  // Used by callers that need a non-rectangular container, e.g. pill-shaped.
  containerClassName?: string;
  /** Open only once the pointer actually moves over the child. Chromium sends
   *  mouseenter (but no mousemove) when content scrolls under a still cursor,
   *  so in a moving chat this keeps cards from popping under a parked mouse. */
  requirePointerMove?: boolean;
  /** The child already carries its name (its own visible text, or an
   *  aria-label), so this tooltip is extra detail: string content goes to
   *  aria-description, the way a native title is read, instead of replacing
   *  the name with aria-label. */
  describe?: boolean;
}

export const Tooltip: React.FC<TooltipProps> = ({
  content,
  side = "top",
  delay = 250,
  children,
  disabled = false,
  containerClassName,
  requirePointerMove = false,
  describe = false,
}) => {
  // Actions are stable for the store's lifetime, so these selectors never
  // re-render. (Reading them via useTooltipStore.getState() at render level
  // reads as "passing a hook around" to the compiler, which then skips the
  // component.) The hover-delay timer lives in the store, keyed by this id,
  // so the component holds no ref and every handler below is a plain
  // event handler.
  const showTooltip = useTooltipStore((s) => s.showTooltip);
  const hideTooltip = useTooltipStore((s) => s.hideTooltip);
  const scheduleShow = useTooltipStore((s) => s.scheduleShow);
  const cancelShow = useTooltipStore((s) => s.cancelShow);
  const isArmed = useTooltipStore((s) => s.isArmed);
  const updateContent = useTooltipStore((s) => s.updateContent);
  const tooltipId = useId();

  useEffect(() => {
    return () => {
      cancelShow(tooltipId);
      hideTooltip(tooltipId);
    };
  }, [cancelShow, hideTooltip, tooltipId]);

  useEffect(() => {
    if (disabled || !content) {
      cancelShow(tooltipId);
      hideTooltip(tooltipId);
    }
  }, [disabled, content, cancelShow, hideTooltip, tooltipId]);

  // A label that changes while it shows (a toggle right after its click)
  // updates in place, as a native title did. Strings only: rich content is a
  // new element on every parent render and would re-measure the card each time.
  useEffect(() => {
    if (typeof content === 'string' && content) updateContent(tooltipId, content);
  }, [content, updateContent, tooltipId]);

  // Touch has no hover: on mobile every tooltip site renders its child
  // untouched (a synthesized mouseenter on tap would otherwise pop tooltips).
  if (IS_MOBILE || disabled || !content) {
    return children;
  }

  const arm = (targetElement: HTMLElement) => {
    scheduleShow(tooltipId, delay, () => {
      const rect = targetElement.getBoundingClientRect();
      showTooltip(tooltipId, content, rect, side, containerClassName);
      // It's okay, if we hover over something else, the store overrides it.
    });
  };

  const handleMouseEnter = (e: React.MouseEvent) => {
    // Call original handler if exists
    if (children.props.onMouseEnter) {
      children.props.onMouseEnter(e);
    }
    if (requirePointerMove) return;

    // Store reference to element synchronously because e.currentTarget becomes null after event bubbling
    arm(e.currentTarget as HTMLElement);
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (children.props.onMouseMove) {
      children.props.onMouseMove(e);
    }
    if (!isArmed(tooltipId)) arm(e.currentTarget as HTMLElement);
  };

  // React never calls onMouseEnter or onMouseMove on a disabled button,
  // input, select or textarea, though Chromium sends the events; the pointer
  // versions still arrive. A disabled child arms on those too, so a tooltip
  // saying why a control is off can show at all.
  const armsOnPointer = children.props.disabled === true;

  const handlePointerEnter = (e: React.PointerEvent) => {
    children.props.onPointerEnter?.(e);
    if (e.pointerType === 'touch' || requirePointerMove) return;
    arm(e.currentTarget as HTMLElement);
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    children.props.onPointerMove?.(e);
    if (e.pointerType !== 'touch' && !isArmed(tooltipId)) arm(e.currentTarget as HTMLElement);
  };

  const handleMouseLeave = (e: React.MouseEvent) => {
    if (children.props.onMouseLeave) {
      children.props.onMouseLeave(e);
    }

    cancelShow(tooltipId);
    hideTooltip(tooltipId);
  };

  const handleFocus = (e: React.FocusEvent) => {
    if (children.props.onFocus) {
      children.props.onFocus(e);
    }
    const targetElement = e.currentTarget as HTMLElement;
    const rect = targetElement.getBoundingClientRect();
    showTooltip(tooltipId, content, rect, side, containerClassName);
  };

  const handleBlur = (e: React.FocusEvent) => {
    if (children.props.onBlur) {
      children.props.onBlur(e);
    }
    hideTooltip(tooltipId);
  };

  // A string tooltip names a child that has no name of its own, or describes
  // one that does.
  const accessible =
    typeof content !== 'string'
      ? null
      : describe
        ? content !== children.props['aria-label'] && !children.props['aria-description']
          ? { 'aria-description': content }
          : null
        : !children.props['aria-label']
          ? { 'aria-label': content }
          : null;

  return React.cloneElement(children, {
    onMouseEnter: handleMouseEnter,
    onMouseLeave: handleMouseLeave,
    ...(requirePointerMove && { onMouseMove: handleMouseMove }),
    ...(armsOnPointer && { onPointerEnter: handlePointerEnter }),
    ...(armsOnPointer && requirePointerMove && { onPointerMove: handlePointerMove }),
    onFocus: handleFocus,
    onBlur: handleBlur,
    ...accessible,
    // Remove title attribute to prevent native tooltip
    title: undefined,
  });
};
