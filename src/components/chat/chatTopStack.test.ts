// The top-of-chat stack meets Rust through `chat_top_stack`'s serialized
// request and answer, so a renamed field fails silently (Tauri refuses the
// argument, or a missing field reads as no cards open). These pin the names
// to the Rust structs, the cards to the stack, and the tap rules.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { toggleCard } from './chatTopStack';

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
      'src/mobile/watch/DropProgressBar.tsx',
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
    expect(phone).toContain('stack={topStack.control.drop}');
    // The drop card left the floating header, which used to paint over the poll.
    const header = phone.slice(phone.indexOf('<PinnedBanner') - 400, phone.indexOf('<PinnedBanner'));
    expect(header).not.toContain('DropProgressBar');
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
