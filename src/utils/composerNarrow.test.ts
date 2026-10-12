import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  COMPOSER_NARROW_PX,
  isNarrowComposer,
  syncComposerPlaceholder,
  trackComposerWidth,
} from './composerNarrow';

/** A composer row and its field, as far as the narrow code reads them. */
function fakeRow(width: number, field: HTMLTextAreaElement | null) {
  return {
    clientWidth: width,
    dataset: {} as Record<string, string>,
    querySelector: () => field,
  } as unknown as HTMLElement;
}

function fakeField(wide: string, narrow: string) {
  return {
    placeholder: wide,
    dataset: { placeholderWide: wide, placeholderNarrow: narrow } as Record<string, string>,
  } as unknown as HTMLTextAreaElement;
}

describe('isNarrowComposer', () => {
  it('compresses a row narrower than the threshold, never a hidden one', () => {
    // The live report: a 262 px chat column left the field 76 px wide.
    expect(isNarrowComposer(246)).toBe(true);
    expect(isNarrowComposer(COMPOSER_NARROW_PX - 1)).toBe(true);
    expect(isNarrowComposer(COMPOSER_NARROW_PX)).toBe(false);
    expect(isNarrowComposer(400)).toBe(false);
    expect(isNarrowComposer(0)).toBe(false);
  });
});

describe('trackComposerWidth', () => {
  const observers: Array<{ cb: () => void; disconnect: ReturnType<typeof vi.fn> }> = [];
  afterEach(() => {
    observers.length = 0;
    vi.unstubAllGlobals();
  });

  it('keeps data-narrow in step with the row width and stops on cleanup', () => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        disconnect = vi.fn();
        constructor(cb: () => void) {
          observers.push({ cb, disconnect: this.disconnect });
        }
        observe() {}
      },
    );
    const field = fakeField('Send a message', 'Message');
    const row = fakeRow(246, field);
    const stop = trackComposerWidth(row);
    expect('narrow' in row.dataset).toBe(true);
    // The live report: "Send a mes", cut with no ellipsis at 262 px.
    expect(field.placeholder).toBe('Message');

    (row as { clientWidth: number }).clientWidth = 380;
    observers[0].cb();
    expect('narrow' in row.dataset).toBe(false);
    expect(field.placeholder).toBe('Send a message');

    stop?.();
    expect(observers[0].disconnect).toHaveBeenCalled();
  });

  it('re-applies the short prompt after the composer writes a new full one', () => {
    const field = fakeField('Send a message', 'Message');
    const row = fakeRow(246, field);
    row.dataset.narrow = '';
    // A render changed the prompt (sub-only came on) and wrote it in full.
    field.dataset.placeholderWide = 'Subscriber-Only Mode';
    field.dataset.placeholderNarrow = 'Sub-only';
    field.placeholder = 'Subscriber-Only Mode';
    syncComposerPlaceholder(row);
    expect(field.placeholder).toBe('Sub-only');
  });

  it('keeps an empty prompt empty and leaves a row with no field alone', () => {
    const field = fakeField('', '');
    const row = fakeRow(246, field);
    row.dataset.narrow = '';
    syncComposerPlaceholder(row);
    expect(field.placeholder).toBe('');
    expect(() => syncComposerPlaceholder(fakeRow(246, null))).not.toThrow();
  });

  it('does nothing for a row that is gone', () => {
    expect(trackComposerWidth(null)).toBeUndefined();
  });
});
