import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../../hooks/usePausedNewCount', () => ({ usePausedNewCount: () => 0 }));

import { PausedOverlay } from './ResumeLiveButton';

describe('the paused overlay', () => {
  it('says the reply hint words it is given', () => {
    const html = renderToStaticMarkup(
      <PausedOverlay paused sliceKey="x" hint hintWords="Swipe left to reply" lift={0} onResume={() => {}} />,
    );
    expect(html).toContain('>Swipe left to reply<');
    expect(html).not.toContain('Tap a message to reply');
  });

  it('shows no hint once it is learned', () => {
    const html = renderToStaticMarkup(
      <PausedOverlay paused sliceKey="x" hint={false} hintWords="Swipe left to reply" lift={0} onResume={() => {}} />,
    );
    expect(html).not.toContain('Swipe left to reply');
  });
});
