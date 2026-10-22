import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Tooltip, type TooltipChildProps, type TooltipProps } from './Tooltip';
import { useTooltipStore } from '../../stores/TooltipStore';

// Renders <Tooltip> around a probe and returns the props the clone handed it.
function cloned(tip: Omit<TooltipProps, 'children'>, child: TooltipChildProps = {}): TooltipChildProps {
  let seen: TooltipChildProps = {};
  const Probe = (props: TooltipChildProps) => {
    seen = props;
    return null;
  };
  renderToStaticMarkup(
    <Tooltip {...tip}>
      <Probe {...child} />
    </Tooltip>,
  );
  return seen;
}

const rect = { left: 0, top: 0, right: 10, bottom: 10, width: 10, height: 10 } as DOMRect;
const pointer = (pointerType: string) =>
  ({ pointerType, currentTarget: { getBoundingClientRect: () => rect } }) as unknown as React.PointerEvent;

describe('Tooltip', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useTooltipStore.getState().hideTooltip();
  });
  afterEach(() => vi.useRealTimers());

  it('names a child that has no name and drops its native title', () => {
    const p = cloned({ content: 'Close' }, { title: 'Close' });
    expect(p['aria-label']).toBe('Close');
    expect(p.title).toBeUndefined();
  });

  it('describes a child that shows its own text instead of renaming it', () => {
    const p = cloned({ content: 'Put this in the chat box', describe: true });
    expect(p['aria-label']).toBeUndefined();
    expect(p['aria-description']).toBe('Put this in the chat box');
  });

  it('does not repeat a description that matches the aria-label', () => {
    const p = cloned({ content: 'Remove the cheer', describe: true }, { 'aria-label': 'Remove the cheer' });
    expect(p['aria-description']).toBeUndefined();
  });

  it('arms a disabled child on pointer events, which React still delivers', () => {
    const p = cloned({ content: 'Sign in to send', delay: 250 }, { disabled: true });
    expect(p.onPointerEnter).toBeTypeOf('function');
    p.onPointerEnter?.(pointer('mouse'));
    vi.advanceTimersByTime(250);
    expect(useTooltipStore.getState().isVisible).toBe(true);
    expect(useTooltipStore.getState().content).toBe('Sign in to send');
  });

  it('ignores a touch on a disabled child', () => {
    const p = cloned({ content: 'Sign in to send' }, { disabled: true });
    p.onPointerEnter?.(pointer('touch'));
    vi.advanceTimersByTime(1000);
    expect(useTooltipStore.getState().isVisible).toBe(false);
  });

  it('leaves an enabled child on mouse events', () => {
    expect(cloned({ content: 'Close' }).onPointerEnter).toBeUndefined();
  });
});
