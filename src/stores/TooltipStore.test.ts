import { beforeEach, describe, expect, it } from 'vitest';
import { useTooltipStore } from './TooltipStore';

const rect = { left: 0, top: 0, right: 10, bottom: 10, width: 10, height: 10 } as DOMRect;

describe('TooltipStore.updateContent', () => {
  beforeEach(() => useTooltipStore.getState().hideTooltip());

  it('swaps the text of the tooltip on screen', () => {
    const s = useTooltipStore.getState();
    s.showTooltip('a', 'Show chat replay', rect);
    s.updateContent('a', 'Hide chat replay');
    expect(useTooltipStore.getState().content).toBe('Hide chat replay');
  });

  it('leaves the tooltip of another trigger alone', () => {
    const s = useTooltipStore.getState();
    s.showTooltip('a', 'Keep on top', rect);
    s.updateContent('b', 'Close');
    expect(useTooltipStore.getState().content).toBe('Keep on top');
  });

  it('does nothing while hidden', () => {
    const s = useTooltipStore.getState();
    s.showTooltip('a', 'One', rect);
    s.hideTooltip('a');
    s.updateContent('a', 'Two');
    expect(useTooltipStore.getState().content).toBe('One');
  });
});
