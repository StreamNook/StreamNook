// Run with: npm test
//
// The emote button's deck and the teaching placeholder: when the empty box
// may teach, which hint shows, which face the button wears, which cards fan,
// and that every composer wires these in (the call sites, not only helpers).

import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));

import {
  deckFace,
  desktopHints,
  fanCards,
  gifLinkInBox,
  hintAt,
  hintCycle,
  hintOfRun,
  hintsMayMove,
  LINKISH,
  mayHoldGifLink,
  nextHintEngaged,
  nextHintRun,
  phoneHints,
  type HintRun,
} from './composerHints';
import { composerLook, type ComposerSituation } from './composerParts';

const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', p), 'utf8');

describe('hint cycle', () => {
  it('teaches what the menu holds, after the prompt', () => {
    expect(hintCycle({ slash: true, gifs: true })).toEqual(['message', 'slash', 'gif']);
    expect(hintCycle({ slash: true, gifs: false })).toEqual(['message', 'slash']);
    expect(hintCycle({ slash: false, gifs: true })).toEqual(['message', 'gif']);
    expect(hintCycle({ slash: false, gifs: false })).toEqual(['message']);
  });

  it('moves one hint per step and wraps', () => {
    const c = hintCycle({ slash: true, gifs: true });
    expect([0, 1, 2, 3, 4].map((s) => hintAt(c, s))).toEqual(['message', 'slash', 'gif', 'message', 'slash']);
    expect(hintAt(c, -3)).toBe('message');
  });

  it('a run starts from the prompt and holds it past the first tick (which may come at once)', () => {
    const c = hintCycle({ slash: true, gifs: true });
    let run: HintRun = { key: null, start: 0 };
    expect(hintOfRun(run, c, 41)).toBe('message');
    run = nextHintRun(run, 'twitch:a|message,slash,gif', 41);
    expect(hintOfRun(run, c, 41)).toBe('message');
    expect(hintOfRun(run, c, 42)).toBe('message');
    expect(hintOfRun(run, c, 43)).toBe('slash');
    expect(hintOfRun(run, c, 44)).toBe('gif');
    expect(hintOfRun(run, c, 45)).toBe('message');
    // Same key: the run goes on.
    expect(nextHintRun(run, 'twitch:a|message,slash,gif', 44)).toBe(run);
    // A Kick chat in the same composer starts over: no GIF hint carried in.
    const kick = hintCycle({ slash: true, gifs: false });
    run = nextHintRun(run, 'kick:a|message,slash', 44);
    expect(hintOfRun(run, kick, 44)).toBe('message');
    expect(hintOfRun(run, kick, 45)).toBe('message');
    // Resting (no key) shows the prompt, and the next run starts fresh.
    run = nextHintRun(run, null, 46);
    expect(hintOfRun(run, kick, 46)).toBe('message');
    run = nextHintRun(run, 'kick:a|message,slash', 50);
    expect(hintOfRun(run, kick, 51)).toBe('message');
    expect(hintOfRun(run, kick, 52)).toBe('slash');
  });

  it('the box counts as written in from anything in it until the composer is left', () => {
    expect(nextHintEngaged(false, '')).toBe(false);
    expect(nextHintEngaged(false, 'h')).toBe(true);
    // Sent: the box empties, the hints keep resting.
    expect(nextHintEngaged(true, '')).toBe(true);
    // Left the composer with an empty box: they may teach again.
    expect(nextHintEngaged(true, '', true)).toBe(false);
    // Left with text in the box: still written in.
    expect(nextHintEngaged(true, 'draft', true)).toBe(true);
  });
});

describe('the window clock', () => {
  it('ticks every hint-length while a box listens and the window shows, and stops after', async () => {
    vi.resetModules();
    vi.useFakeTimers();
    const vis = { hidden: false, cb: null as null | (() => void) };
    vi.doMock('../../utils/windowVisibility', () => ({
      isWindowHidden: () => vis.hidden,
      onWindowVisibility: (cb: () => void) => {
        vis.cb = cb;
        return () => {
          vis.cb = null;
        };
      },
    }));
    const g = globalThis as unknown as { window?: unknown };
    const hadWindow = 'window' in g;
    if (!hadWindow) g.window = globalThis;
    try {
      const clock = await import('./composerHints');
      const ticks: number[] = [];
      const off = clock.subscribeHintClock(() => ticks.push(clock.hintClockStep()));
      const start = clock.hintClockStep();
      vi.advanceTimersByTime(clock.HINT_MS - 1);
      expect(ticks).toEqual([]);
      vi.advanceTimersByTime(1);
      expect(ticks).toEqual([start + 1]);
      // Hidden: no ticks.
      vis.hidden = true;
      vis.cb?.();
      vi.advanceTimersByTime(clock.HINT_MS * 3);
      expect(ticks).toEqual([start + 1]);
      // Shown again: ticking again.
      vis.hidden = false;
      vis.cb?.();
      vi.advanceTimersByTime(clock.HINT_MS);
      expect(ticks).toEqual([start + 1, start + 2]);
      // The last listener gone: the interval is cleared.
      off();
      expect(vis.cb).toBeNull();
      vi.advanceTimersByTime(clock.HINT_MS * 3);
      expect(ticks).toEqual([start + 1, start + 2]);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      if (!hadWindow) delete g.window;
      vi.doUnmock('../../utils/windowVisibility');
      vi.useRealTimers();
    }
  });
});

describe('motion', () => {
  it('moves only at full motion and never while quieted', () => {
    expect(hintsMayMove('full', false)).toBe(true);
    expect(hintsMayMove(undefined, false)).toBe(true);
    expect(hintsMayMove('reduced', false)).toBe(false);
    expect(hintsMayMove('off', false)).toBe(false);
    expect(hintsMayMove('full', true)).toBe(false);
  });
});

describe('desktop gate', () => {
  const idle = {
    teaches: true,
    empty: true,
    engaged: false,
    slowWait: 0,
    replying: false,
    menuOpen: false,
    remindOverlay: false,
    motionMode: 'full',
    lowPower: false,
  };

  it('teaches over an idle empty box', () => {
    expect(desktopHints(idle)).toEqual({ layer: true, active: true });
  });

  it('never stands over a prompt that says something, a box with text, a /remind, or still motion', () => {
    for (const s of [
      { teaches: false },
      { empty: false },
      { remindOverlay: true },
      { motionMode: 'reduced' },
      { motionMode: 'off' },
      { lowPower: true },
    ]) {
      expect(desktopHints({ ...idle, ...s })).toEqual({ layer: false, active: false });
    }
  });

  it('rests on the prompt while you write, wait out slow mode, reply or browse the menu', () => {
    for (const s of [{ engaged: true }, { slowWait: 3 }, { replying: true }, { menuOpen: true }]) {
      expect(desktopHints({ ...idle, ...s })).toEqual({ layer: true, active: false });
    }
  });
});

describe('phone gate', () => {
  const idle = {
    gifs: true,
    plainPrompt: true,
    empty: true,
    typing: false,
    panelOpen: false,
    motionMode: 'full',
    lowPower: false,
  };

  it('teaches GIFs over an idle empty box', () => {
    expect(phoneHints(idle)).toEqual({ layer: true, active: true });
  });

  it('teaches nothing where no GIFs are offered or the prompt says something', () => {
    for (const s of [{ gifs: false }, { plainPrompt: false }, { empty: false }, { motionMode: 'reduced' }, { lowPower: true }]) {
      expect(phoneHints({ ...idle, ...s }).layer).toBe(false);
    }
  });

  it('rests while the keyboard is up or a panel is open', () => {
    expect(phoneHints({ ...idle, typing: true })).toEqual({ layer: true, active: false });
    expect(phoneHints({ ...idle, panelOpen: true })).toEqual({ layer: true, active: false });
  });
});

describe('face and fan', () => {
  const base = { text: '', gifLink: false, hint: 'message' as const, slash: true, gifs: true };

  it('what the box holds wins, then the hint, else the moment', () => {
    expect(deckFace({ ...base, text: '/ban' })).toBe('slash');
    expect(deckFace({ ...base, text: 'look https://giphy.com/x', gifLink: true })).toBe('gif');
    expect(deckFace({ ...base, hint: 'slash' })).toBe('slash');
    expect(deckFace({ ...base, hint: 'gif' })).toBe('gif');
    expect(deckFace(base)).toBe('moment');
    expect(deckFace({ ...base, text: 'hello', hint: 'gif' })).toBe('moment');
  });

  it('never shows what the menu does not hold', () => {
    expect(deckFace({ ...base, slash: false, text: '/ban' })).toBe('moment');
    expect(deckFace({ ...base, gifs: false, gifLink: true, text: 'x' })).toBe('moment');
    expect(deckFace({ ...base, gifs: false, hint: 'gif' })).toBe('moment');
  });

  it('fans the deck\'s other members, keeping each on its side', () => {
    expect(fanCards('moment', { slash: true, gifs: true })).toEqual(['gif', 'slash']);
    expect(fanCards('slash', { slash: true, gifs: true })).toEqual(['moment', 'gif']);
    expect(fanCards('gif', { slash: true, gifs: true })).toEqual(['moment', 'slash']);
    // Off Twitch only the slash card fans.
    expect(fanCards('moment', { slash: true, gifs: false })).toEqual(['slash']);
  });

  it('asks Rust only about text that could hold a link, and keeps the face while the link stays', () => {
    expect(mayHoldGifLink('hello there')).toBe(false);
    expect(mayHoldGifLink('see https://tenor.com/v/1')).toBe(true);
    expect(mayHoldGifLink('gph.is/abc')).toBe(true);
    expect(gifLinkInBox('https://tenor.com/v/1', 'see https://tenor.com/v/1 lol')).toBe(true);
    expect(gifLinkInBox('https://tenor.com/v/1', 'see ')).toBe(false);
    expect(gifLinkInBox(null, 'https://tenor.com/v/1')).toBe(false);
  });

  it('the cheap gate lets every GIF host Rust knows through, with or without a scheme', () => {
    const rust = src('../src-tauri/src/services/link_detect.rs');
    const list = rust.match(/const GIF_HOSTS: &\[&str\] = &\[([^\]]*)\]/);
    expect(list).not.toBeNull();
    const hosts = [...list![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(hosts.length).toBeGreaterThan(0);
    for (const h of hosts) {
      expect(LINKISH.test(`${h}/x`), h).toBe(true);
      expect(LINKISH.test(`media.${h}/x`), h).toBe(true);
    }
  });

  it('a null answer clears a found link, and the probe runs only with the button', () => {
    const h = src('components/chat/composerHints.ts');
    expect(h).toContain('if (alive) setFound(link ?? null);');
    const d = src('components/chat/ComposerDeck.tsx');
    expect(d).toContain('useGifLinkInBox(text, gifs && showButton)');
  });
});

describe('composerLook teaches', () => {
  const s: ComposerSituation = {
    canSend: true,
    provider: 'twitch',
    signInOwed: false,
    watchStreak: false,
    resub: false,
    subOnlyBlocked: false,
    inputDisabled: false,
    roomModes: null,
  };
  const modes = (extra: object) => ({
    modes: [{ kind: 'followers_only' as const, label: 'Followers-only chat', compact: 'Followers-only', short: 'Followers' }],
    chip: 'Followers',
    exempt: false,
    prompt: 'Followers-only chat',
    prompt_short: 'Followers',
    standing: null,
    slow_seconds: 0,
    ...extra,
  });

  it('only over the idle prompt', () => {
    expect(composerLook(undefined, s).teaches).toBe(true);
    expect(composerLook(undefined, { ...s, canSend: false }).teaches).toBe(false);
    expect(composerLook(undefined, { ...s, signInOwed: true }).teaches).toBe(false);
    expect(composerLook(undefined, { ...s, watchStreak: true }).teaches).toBe(false);
    expect(composerLook(undefined, { ...s, resub: true }).teaches).toBe(false);
    expect(composerLook(undefined, { ...s, subOnlyBlocked: true }).teaches).toBe(false);
    expect(composerLook(undefined, { ...s, inputDisabled: true }).teaches).toBe(false);
    expect(composerLook({ hide_placeholder: true } as never, s).teaches).toBe(false);
  });

  it('a room mode in the prompt wins; as a chip it still stops the hints when it blocks or warns', () => {
    const prompt = { ...s, roomModes: modes({}) as never };
    expect(composerLook(undefined, prompt).teaches).toBe(false);
    const chip = { room_modes_style: 'chip' } as never;
    expect(composerLook(chip, prompt).teaches).toBe(true);
    expect(composerLook(chip, { ...s, roomModes: modes({ blocked: 'followers_only' }) as never }).teaches).toBe(false);
    expect(composerLook(chip, { ...s, roomModes: modes({ warning: { title: 'x' } }) as never }).teaches).toBe(false);
  });
});

describe('call sites', () => {
  it('chat\'s composer gates the hints in one place and leaves the clock to the leaves', () => {
    const w = src('components/ChatWidget.tsx');
    expect(w).toContain('desktopHints({');
    expect(w).toContain('<ComposerDeck');
    expect(w).toContain('gate={hintGate}');
    expect(w).toContain("data-hint-layer={hintGate.layer ? '' : undefined}");
    expect(w).toContain('gifs={gifsOffered && !emoteOnlyRoom}');
    expect(w).toContain('engaged: hintEngaged,');
    expect(w).toContain('const engagedNow = nextHintEngaged(hintEngaged, messageInput);');
    expect(w).toContain('if (engagedNow !== hintEngaged) setHintEngaged(engagedNow);');
    // Leaving the composer row, not the field alone, lets the box teach again.
    expect(w).toContain('if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {');
    expect(w).toContain('setHintEngaged((prev) => nextHintEngaged(prev, messageInput, true));');
    // The clock never lives in the big component.
    expect(w).not.toContain('useComposerHint(');
    expect(w).not.toContain('useEmoteMoment(');
  });

  it('the phone gates the hints the phone way and draws the leaves', () => {
    const p = src('mobile/chat/MobileChatInput.tsx');
    expect(p).toContain('phoneHints({');
    expect(p).toContain('<RoomDeckFace');
    expect(p).toContain('<PhoneHintLayer');
    expect(p).toContain("data-hint-layer={hintGate.layer ? '' : undefined}");
    expect(p).not.toContain('useComposerHint(');
  });

  it('the deck asks for the moment only while its button shows', () => {
    const d = src('components/chat/ComposerDeck.tsx');
    const button = d.slice(d.indexOf('function DeckButton('), d.indexOf('export function ComposerDeck('));
    expect(button).toContain('useEmoteMoment(');
    const deck = d.slice(d.indexOf('export function ComposerDeck('), d.indexOf('export function RoomDeckFace('));
    expect(deck).not.toContain('useEmoteMoment(');
    expect(deck).toContain('{showButton && (');
  });

  it('the face stills an animated emote wherever motion or chat\'s emote setting asks', () => {
    const b = src('components/chat/EmoteDeckButton.tsx');
    expect(b).toContain("return still || animate !== 'always' || motion !== 'full';");
    const d = src('components/chat/ComposerDeck.tsx');
    // Hovering or focusing the deck holds the hint it arrived on.
    expect(d).toContain("const hint: HintKind = !gate.active ? 'message' : holding ? (held ?? running) : running;");
  });

  it('hides the field\'s own placeholder under the layer, beating the placeholder ink classes', () => {
    const css = src('styles/globals.css');
    expect(css).toMatch(/textarea\[data-hint-layer\]::placeholder\s*\{\s*color: transparent;/);
    expect(css).toMatch(/html\[data-motion='off'\] \.sn-deck-card,\s*html\[data-sn-perf\] \.sn-deck-card \{\s*transition: none;/);
  });
});
