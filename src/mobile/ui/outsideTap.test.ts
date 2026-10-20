import { describe, expect, it, vi } from 'vitest';
import { editingField, installKeyboardTaps, isTap, keyboardTap } from './outsideTap';

/** A stand-in element: its tag, the marks it sits under, and what it contains. */
function el(tag: string, marks: string[] = [], opts: { type?: string; inside?: unknown[] } = {}) {
  const node = {
    tagName: tag,
    type: opts.type,
    isContentEditable: false,
    blur: vi.fn(),
    contains: (other: unknown) => other === node || (opts.inside ?? []).includes(other),
    closest: (selector: string) =>
      selector.split(',').some((part) => marks.some((m) => part.trim() === `[${m}]`)) ? node : null,
    getBoundingClientRect: () => ({ top: 700 }),
  };
  return node as unknown as HTMLElement & { blur: ReturnType<typeof vi.fn> };
}

/** A document the rule listens on, with the field that has the keyboard. */
function page(active: HTMLElement | null, composer = el('DIV', ['data-chat-composer'])) {
  const doc = new EventTarget() as EventTarget & { activeElement: Element | null; querySelector: () => Element };
  doc.activeElement = active;
  doc.querySelector = () => composer;
  return doc;
}

function fire<T extends Event>(doc: EventTarget, type: string, props: Record<string, unknown>): T {
  const e = new Event(type, { cancelable: true, bubbles: true }) as T;
  for (const [k, v] of Object.entries(props)) Object.defineProperty(e, k, { value: v });
  doc.dispatchEvent(e);
  return e;
}

describe('keyboardTap', () => {
  const field = el('TEXTAREA', ['data-chat-composer']);
  it('a tap on the chat puts the keyboard away and stops there', () => {
    expect(keyboardTap(el('DIV'), field)).toBe('use');
  });
  it('the composer is the field’s own: nothing goes', () => {
    expect(keyboardTap(el('BUTTON', ['data-chat-composer']), field)).toBe('none');
    expect(keyboardTap(el('BUTTON', ['data-keeps-keyboard']), field)).toBe('none');
    expect(keyboardTap(field, field)).toBe('none');
  });
  it('a search result or a sheet button takes the tap as the keyboard goes', () => {
    expect(keyboardTap(el('BUTTON', ['data-taps-through-keyboard']), field)).toBe('pass');
  });
  it('without a keyboard a tap is just a tap', () => {
    expect(keyboardTap(el('DIV'), null)).toBe('none');
  });
});

describe('editingField', () => {
  it('knows the fields that bring a keyboard', () => {
    expect(editingField(el('TEXTAREA'))).not.toBeNull();
    expect(editingField(el('INPUT', [], { type: 'search' }))).not.toBeNull();
    expect(editingField(el('INPUT', [], { type: 'range' }))).toBeNull();
    expect(editingField(el('BUTTON'))).toBeNull();
    expect(editingField(null)).toBeNull();
  });
});

describe('isTap', () => {
  it('a drag is not a tap', () => {
    expect(isTap(3, 4)).toBe(true);
    expect(isTap(0, 24)).toBe(false);
  });
});

describe('installKeyboardTaps', () => {
  it('a tap on a chat row blurs the field and the row never hears the click', () => {
    const field = el('TEXTAREA', ['data-chat-composer']);
    const doc = page(field);
    const stop = installKeyboardTaps(doc as unknown as Document);
    const row = el('DIV', ['data-chat-list']);
    fire(doc, 'pointerdown', { target: row, pointerId: 1, clientX: 100, clientY: 200 });
    const click = fire<MouseEvent>(doc, 'click', { target: row });
    expect(field.blur).toHaveBeenCalled();
    expect(click.defaultPrevented).toBe(true);
    stop();
  });

  it('a scroll closes nothing', () => {
    const field = el('TEXTAREA', ['data-chat-composer']);
    const doc = page(field);
    const stop = installKeyboardTaps(doc as unknown as Document);
    const row = el('DIV', ['data-chat-list']);
    fire(doc, 'pointerdown', { target: row, pointerId: 1, clientX: 100, clientY: 200 });
    fire(doc, 'pointercancel', { target: row, pointerId: 1 });
    const click = fire<MouseEvent>(doc, 'click', { target: row });
    expect(field.blur).not.toHaveBeenCalled();
    expect(click.defaultPrevented).toBe(false);
    stop();
  });

  it('a search result opens on the tap that puts the keyboard away', () => {
    const field = el('INPUT', [], { type: 'search' });
    const doc = page(field);
    const stop = installKeyboardTaps(doc as unknown as Document);
    const result = el('BUTTON', ['data-taps-through-keyboard']);
    fire(doc, 'pointerdown', { target: result, pointerId: 2, clientX: 50, clientY: 300 });
    const click = fire<MouseEvent>(doc, 'click', { target: result });
    expect(field.blur).toHaveBeenCalled();
    expect(click.defaultPrevented).toBe(false);
    stop();
  });

  it('a drag down the chat list that reaches the keyboard pulls it down', () => {
    const field = el('TEXTAREA', ['data-chat-composer']);
    const doc = page(field);
    const stop = installKeyboardTaps(doc as unknown as Document, () => 700);
    const row = el('DIV', ['data-chat-list']);
    fire(doc, 'touchstart', { target: row, touches: [{ clientY: 300 }] });
    fire(doc, 'touchmove', { target: row, touches: [{ clientY: 650 }] });
    expect(field.blur).not.toHaveBeenCalled();
    fire(doc, 'touchmove', { target: row, touches: [{ clientY: 705 }] });
    expect(field.blur).toHaveBeenCalled();
    stop();
  });
});
