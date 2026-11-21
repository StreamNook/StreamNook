// The top-of-chat stack meets Rust through `chat_top_stack`'s serialized
// request and answer, so a renamed field fails silently (Tauri refuses the
// argument, or a missing field reads as no cards open). These pin the names
// to the Rust structs, the cards to the stack, and the tap rules.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { choiceKey, slotControl, toggleCard, type StackLayout, type StackRequest } from './chatTopStack';

// Hoisted above the import: the hook's modules need no app to load.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../../stores/AppStore', () => ({ useAppStore: vi.fn() }));
vi.mock('../../utils/logger', () => ({ Logger: { debug: vi.fn() } }));

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');
const RUST = 'src-tauri/src/services/chat_top_stack.rs';
const TS = 'src/components/chat/chatTopStack.ts';

function rustFields(name: string): string[] {
  const m = read(RUST).match(new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`));
  if (!m) throw new Error(`${name} not found`);
  return [...m[1].matchAll(/^\s*pub (\w+):/gm)].map((x) => x[1]).sort();
}

function tsFields(name: string): string[] {
  const m = read(TS).match(new RegExp(`export interface ${name} \\{([\\s\\S]*?)\\n\\}`));
  if (!m) throw new Error(`${name} not found`);
  return [...m[1].matchAll(/^ {2}(\w+)\??:/gm)].map((x) => x[1]).sort();
}

describe('chat top stack: the TS shapes match Rust', () => {
  for (const name of ['StackRequest', 'StackLayout', 'StackSlot']) {
    it(name, () => expect(tsFields(name)).toEqual(rustFields(name)));
  }

  it('invokes the command with its one Rust parameter', () => {
    const cmd = read('src-tauri/src/commands/drops.rs').match(/pub fn chat_top_stack\(([\s\S]*?)\)\s*->/);
    expect(cmd?.[1].trim().split(':')[0]).toBe('request');
    expect(read(TS)).toMatch(/invoke<StackLayout>\('chat_top_stack', \{ request \}\)/);
  });

  it('every card of the stack reports itself and draws in its slot', () => {
    for (const file of [
      'src/components/PollOverlay.tsx',
      'src/components/PredictionOverlay.tsx',
    ]) {
      const src = read(file);
      expect(src, file).toContain('useStackPresence(stack,');
      expect(src, file).toMatch(/stack\.placed/);
    }
  });

  it('both shells lay out their cards with it', () => {
    const desktop = read('src/components/ChatWidget.tsx');
    expect(desktop).toContain('useChatTopStack(chatContainerEl)');
    expect(desktop).toContain('maxHeight={topStack.maxHeight}');
    const phone = read('src/mobile/screens/WatchScreen.tsx');
    expect(phone).toContain('useChatTopStack(chatColumnEl)');
    // Drops progress is the lip on the composer now, never a card up here.
    expect(phone).not.toContain('topStack.control.drop');
    expect(phone).not.toContain('DropProgressBar');
  });
});

describe('chat top stack: a tap on a head', () => {
  const slots = (open: string | null) =>
    (['poll', 'drop'] as const).map((kind) => ({ kind, open: kind === open }));

  it('opens a strip, folding the card that was open', () => {
    const next = toggleCard(slots('poll'), null, [], 'drop');
    expect(next.opened).toBe('drop');
    expect(next.folded).toEqual(['poll']);
    expect(next.slots.filter((s) => s.open).map((s) => s.kind)).toEqual(['drop']);
  });

  it('folds the open card and leaves nothing open', () => {
    const next = toggleCard(slots('drop'), 'drop', ['poll'], 'drop');
    expect(next.opened).toBeNull();
    expect(next.folded).toEqual(['poll', 'drop']);
    expect(next.slots.some((s) => s.open)).toBe(false);
  });

  it('reopens a folded card', () => {
    const next = toggleCard(slots(null), null, ['poll', 'drop'], 'poll');
    expect(next.opened).toBe('poll');
    expect(next.folded).toEqual(['drop']);
  });
});

describe('accent on cards', () => {
  it('is a theme variable with a Tailwind color', () => {
    expect(read('src/themes/index.ts')).toContain(
      "root.style.setProperty('--color-accent-on-surface', palette.accentOnSurface ?? palette.accent)",
    );
    expect(read('tailwind.config.js')).toContain("'accent-on-surface': 'var(--color-accent-on-surface)'");
  });

  it('the poll card draws no accent fill under its text and uses the floored accent', () => {
    const src = read('src/components/PollOverlay.tsx');
    expect(src).not.toMatch(/bg-accent\//);
    expect(src).not.toMatch(/text-accent(?!-on-surface)/);
    expect(read('src/components/chat/OverlayBanner.tsx')).toContain('bg-accent-on-surface');
  });
});

// The shared case file (SN-284, D21): Rust asserts every layout, and this shell
// reads the same cases, so the request it sends and the answer it consumes
// cannot drift from the rule.
interface StackCase {
  name: string;
  request: Record<string, unknown>;
  layout: StackLayout;
}
const cases = JSON.parse(read('src-tauri/tests/fixtures/chat-top-stack-cases.json')) as StackCase[];

describe('chat top stack: the shared case file', () => {
  it('has cases', () => {
    expect(cases.length).toBeGreaterThan(0);
  });

  it('every request the cases make is one this shell can send, field for field', () => {
    const fields = tsFields('StackRequest');
    for (const c of cases) {
      expect(Object.keys(c.request).sort(), c.name).toEqual(fields);
      // typed: a request built from the case is a StackRequest
      const typed: StackRequest = c.request as unknown as StackRequest;
      expect(Array.isArray(typed.cards)).toBe(true);
    }
  });

  it('every answer is consumed slot for slot: form, height and place', () => {
    for (const c of cases) {
      c.layout.slots.forEach((slot, i) => {
        const control = slotControl(c.layout.slots, slot.kind);
        expect(control.placed, `${c.name} ${slot.kind}`).toBe(true);
        expect(control.open).toBe(slot.open);
        expect(control.form).toBe(slot.form);
        expect(control.height).toBe(slot.height);
        expect(control.order).toBe(i);
        expect(control.moreCards).toBe(slot.more_cards ?? 0);
      });
      // A card the answer does not name is not placed.
      expect(slotControl([], 'poll').placed).toBe(false);
    }
  });

  it('draws an open card that cannot fit its body as a strip (the answer says so)', () => {
    const noRoom = cases.flatMap((c) => c.layout.slots).filter((s) => s.open && s.form === 'strip');
    for (const slot of noRoom) {
      const control = slotControl([slot], slot.kind);
      expect(control.open).toBe(true);
      expect(control.form).toBe('strip');
    }
  });

  it('keeps the open and folded choice by the room key Rust named, and the phones by room only', () => {
    expect(choiceKey('main-chat', 'xqc', false)).toBe('main-chat|xqc');
    expect(choiceKey('dock', 'xqc', false)).not.toBe(choiceKey('main-chat', 'xqc', false));
    expect(choiceKey('phone-watch', 'xqc', true)).toBe(choiceKey('phone-chats', 'xqc', true));
  });
});
