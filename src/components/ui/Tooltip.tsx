import React, { useEffect, useId } from "react";
import { useTooltipStore } from "../../stores/TooltipStore";
import { IS_MOBILE } from "../../utils/platform";

/** The props the tooltip reads from, and injects into, its single child. */
export interface TooltipChildProps {
  onMouseEnter?: (e: React.MouseEvent) => void;
  onMouseLeave?: (e: React.MouseEvent) => void;
  onFocus?: (e: React.FocusEvent) => void;
  onBlur?: (e: React.FocusEvent) => void;
  "aria-label"?: string;
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
}

export const Tooltip: React.FC<TooltipProps> = ({
  content,
  side = "top",
  delay = 250,
  children,
  disabled = false,
  containerClassName,
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

  // Touch has no hover: on mobile every tooltip site renders its child
  // untouched (a synthesized mouseenter on tap would otherwise pop tooltips).
  if (IS_MOBILE || disabled || !content) {
    return children;
  }

  const handleMouseEnter = (e: React.MouseEvent) => {
    // Call original handler if exists
    if (children.props.onMouseEnter) {
      children.props.onMouseEnter(e);
    }

    // Store reference to element synchronously because e.currentTarget becomes null after event bubbling
    const targetElement = e.currentTarget as HTMLElement;

    scheduleShow(tooltipId, delay, () => {
      const rect = targetElement.getBoundingClientRect();
      showTooltip(tooltipId, content, rect, side, containerClassName);
      // It's okay, if we hover over something else, the store overrides it.
    });
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

  return React.cloneElement(children, {
    onMouseEnter: handleMouseEnter,
    onMouseLeave: handleMouseLeave,
    onFocus: handleFocus,
    onBlur: handleBlur,
    // Add aria-label if children don't have one and content is string
    ...(typeof content === 'string' && !children.props['aria-label'] && { 'aria-label': content }),
    // Remove title attribute to prevent native tooltip
    title: undefined,
  });
};
