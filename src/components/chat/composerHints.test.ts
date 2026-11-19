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
  chatSeed,
  deckFace,
  desktopHints,
  fanCards,
  gifLinkInBox,
  hintCycle,
  hintOfVisit,
  hintsMayMove,
  LINKISH,
  mayHoldGifLink,
  nextHintEngaged,
  nextHintVisit,
  phoneHints,
  type HintKind,
  type HintVisit,
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

  it('a chat keeps its hint, and moves to the next one per finished message, wrapping', () => {
    const c = hintCycle({ slash: true, gifs: true });
    const order: HintKind[] = ['message', 'slash', 'gif'];
    const first = hintOfVisit(c, 'twitch:a', 0);
    // The same chat opens on the same hint every visit.
    expect(hintOfVisit(c, 'twitch:a', 0)).toBe(first);
    const at = order.indexOf(first);
    expect([1, 2, 3].map((n) => hintOfVisit(c, 'twitch:a', n))).toEqual([1, 2, 3].map((n) => order[(at + n) % 3]));
    // Chats open on their own hints, not all on the same one.
    const opens = new Set(['twitch:a', 'twitch:b', 'twitch:c', 'twitch:d', 'twitch:e', 'kick:a'].map((k) => hintOfVisit(c, k, 0)));
    expect(opens.size).toBeGreaterThan(1);
    expect(chatSeed('twitch:a')).toBe(chatSeed('twitch:a'));
  });

  it('never shows a hint the menu does not hold, and a lone prompt never changes', () => {
    const kick = hintCycle({ slash: true, gifs: false });
    for (let n = 0; n < 6; n++) expect(hintOfVisit(kick, 'kick:a', n)).not.toBe('gif');
    const bare = hintCycle({ slash: false, gifs: false });
    for (let n = 0; n < 6; n++) expect(hintOfVisit(bare, 'twitch:a', n)).toBe('message');
  });

  it('a pick does not move when GIFs turn out to be offered a moment later', () => {
    const before = hintCycle({ slash: true, gifs: false });
    const after = hintCycle({ slash: true, gifs: true });
    for (const k of ['twitch:a', 'twitch:b', 'twitch:c', 'twitch:d']) {
      const was = hintOfVisit(before, k, 0);
      if (was !== 'message') expect(hintOfVisit(after, k, 0)).toBe(was);
    }
  });

  it('only a new chat or a finished message starts the next hint; typing does not', () => {
    let v: HintVisit = { key: 'twitch:a', filled: false, finished: 0 };
    expect(nextHintVisit(v, 'twitch:a', '')).toBe(v);
    v = nextHintVisit(v, 'twitch:a', 'h');
    expect(v.finished).toBe(0);
    // Further keystrokes keep the same visit object (no render churn).
    expect(nextHintVisit(v, 'twitch:a', 'hello')).toBe(v);
    v = nextHintVisit(v, 'twitch:a', '');
    expect(v.finished).toBe(1);
    // A new chat starts over at its own hint.
    v = nextHintVisit(v, 'kick:b', '');
    expect(v).toEqual({ key: 'kick:b', filled: false, finished: 0 });
  });

  it('nothing in the hints runs on a timer', () => {
    const h = src('components/chat/composerHints.ts');
    expect(h).not.toMatch(/setInterval|setTimeout\([^)]*HINT/);
    expect(h).not.toContain('HINT_MS');
    const d = src('components/chat/ComposerDeck.tsx');
    expect(d).not.toMatch(/setInterval|useSyncExternalStore/);
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

  it('rests on the prompt while you write, wait out slow mode or reply', () => {
    for (const s of [{ engaged: true }, { slowWait: 3 }, { replying: true }]) {
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

  it('rests while the keyboard is up', () => {
    expect(phoneHints({ ...idle, typing: true })).toEqual({ layer: true, active: false });
  });
});

describe('face and fan', () => {
  const base = { text: '', gifLink: false, slash: true, gifs: true };

  it('what the box holds wins, else the moment; an empty box never turns the face', () => {
    expect(deckFace({ ...base, text: '/ban' })).toBe('slash');
    expect(deckFace({ ...base, text: 'look https://giphy.com/x', gifLink: true })).toBe('gif');
    expect(deckFace(base)).toBe('moment');
    expect(deckFace({ ...base, text: 'hello' })).toBe('moment');
  });

  it('never shows what the menu does not hold', () => {
    expect(deckFace({ ...base, slash: false, text: '/ban' })).toBe('moment');
    expect(deckFace({ ...base, gifs: false, gifLink: true, text: 'x' })).toBe('moment');
  });

  it('every face change is a soft cross-fade: opacity only, no pop or rise', () => {
    const b = src('components/chat/EmoteDeckButton.tsx');
    const face = b.slice(b.indexOf('export function DeckFace('), b.indexOf('const FAN'));
    expect(face).toContain('initial={{ opacity: 0 }}');
    expect(face).toContain('exit={{ opacity: 0 }}');
    expect(face).not.toMatch(/scale:|\by:/);
    const l = src('components/chat/ComposerHintLayer.tsx');
    expect(l).toContain('initial={{ opacity: 0 }}');
    expect(l).not.toMatch(/scale:|\by:/);
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
    // The hint reads the box, so a finished message moves it on.
    expect(d).toContain('useComposerHint(gate.active, cycle, roomKeyOf(room), text)');
  });

  it('hides the field\'s own placeholder under the layer, beating the placeholder ink classes', () => {
    const css = src('styles/globals.css');
    expect(css).toMatch(/textarea\[data-hint-layer\]::placeholder\s*\{\s*color: transparent;/);
    expect(css).toMatch(/html\[data-motion='off'\] \.sn-deck-card,\s*html\[data-sn-perf\] \.sn-deck-card \{\s*transition: none;/);
  });
});
