// The Android sign-in overlay presses Kick's own Log In button on kick.com/login
// (KICK_OPEN_LOGIN in TwitchLoginPlugin.kt). Kick's login form is a modal that
// sets `pointer-events: none` on the page body, so a OneTrust cookie banner up
// while the form is open ignored every tap and sign-in was stuck. These run the
// script itself against a small fake page: the banner layer takes taps, the
// press waits for the banner, and a late banner that closed the form opens it
// again. The iPhone's KickLogin.script is the same text.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const kotlin = readFileSync(
  resolve(__dirname, '..', '..', 'src-tauri/gen/android/app/src/main/java/app/streamnook/TwitchLoginPlugin.kt'),
  'utf8',
).replace(/\r\n/g, '\n');

const START = 'private const val KICK_OPEN_LOGIN = """';
const script = (() => {
  const from = kotlin.indexOf(START);
  const to = kotlin.indexOf('"""', from + START.length);
  return from >= 0 && to > from ? kotlin.slice(from + START.length, to) : '';
})();

interface Page {
  banner: boolean;
  panel: boolean;
  formOpen: boolean;
  oneTrust: boolean;
  presses: number;
  styles: string[];
}

function run(page: Page) {
  const shownEl = (visible: () => boolean) => ({ getClientRects: () => (visible() ? [{}] : []) });
  const ids: Record<string, ReturnType<typeof shownEl>> = {
    'onetrust-banner-sdk': shownEl(() => page.banner),
    'onetrust-pc-sdk': shownEl(() => page.panel),
  };
  const document = {
    head: { appendChild: (el: { textContent: string }) => page.styles.push(el.textContent) },
    documentElement: {},
    createElement: () => ({ textContent: '' }),
    getElementById: (id: string) => ids[id] ?? null,
    querySelector: (sel: string) => {
      if (sel === 'input[type="password"]') return page.formOpen ? {} : null;
      if (sel === 'button[data-testid="login"]') {
        return {
          click: () => {
            page.presses += 1;
            page.formOpen = true;
          },
        };
      }
      return null;
    },
  };
  const window = { get OneTrust() { return page.oneTrust ? {} : undefined; } };
  const getComputedStyle = () => ({ visibility: 'visible' });
  new Function('document', 'window', 'getComputedStyle', 'setInterval', 'clearInterval', script)(
    document,
    window,
    getComputedStyle,
    setInterval,
    clearInterval,
  );
}

const fresh = (over: Partial<Page> = {}): Page => ({
  banner: false,
  panel: false,
  formOpen: false,
  oneTrust: true,
  presses: 0,
  styles: [],
  ...over,
});

describe('Kick login press and the cookie banner (Android overlay)', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('reads the script', () => {
    expect(script).toContain("button[data-testid=\"login\"]");
    expect(script).not.toContain('$');
  });

  it('lets the OneTrust layer take taps over Kick\'s dialog', () => {
    const page = fresh();
    run(page);
    expect(page.styles).toEqual(['#onetrust-consent-sdk{pointer-events:auto !important}']);
  });

  it('opens the form at once when no banner shows, and presses once', () => {
    const page = fresh();
    run(page);
    vi.advanceTimersByTime(600);
    expect(page.formOpen).toBe(true);
    vi.advanceTimersByTime(10_000);
    expect(page.presses).toBe(1);
  });

  it('waits while the banner is up, then opens the form once it is answered', () => {
    const page = fresh({ banner: true });
    run(page);
    vi.advanceTimersByTime(30_000);
    expect(page.presses).toBe(0);
    page.banner = false;
    vi.advanceTimersByTime(600);
    expect(page.presses).toBe(1);
    expect(page.formOpen).toBe(true);
  });

  it('waits while the Show Purposes panel is up', () => {
    const page = fresh({ panel: true });
    run(page);
    vi.advanceTimersByTime(10_000);
    expect(page.presses).toBe(0);
    page.panel = false;
    vi.advanceTimersByTime(600);
    expect(page.presses).toBe(1);
  });

  it('opens the form again when answering a late banner closed it', () => {
    const page = fresh();
    run(page);
    vi.advanceTimersByTime(600);
    expect(page.presses).toBe(1);
    page.banner = true;
    vi.advanceTimersByTime(2_000);
    // The tap on the banner lands outside Kick's dialog, which closes it.
    page.banner = false;
    page.formOpen = false;
    vi.advanceTimersByTime(600);
    expect(page.presses).toBe(2);
    expect(page.formOpen).toBe(true);
  });

  it('leaves a form the person closed themselves alone', () => {
    const page = fresh();
    run(page);
    // The press, then a check that sees the form open.
    vi.advanceTimersByTime(1_100);
    page.formOpen = false;
    vi.advanceTimersByTime(10_000);
    expect(page.presses).toBe(1);
  });

  it('gives OneTrust two seconds to load before the first press', () => {
    const page = fresh({ oneTrust: false });
    run(page);
    vi.advanceTimersByTime(1_600);
    expect(page.presses).toBe(0);
    vi.advanceTimersByTime(500);
    expect(page.presses).toBe(1);
  });

  it('stops after 20 presses when the form never opens', () => {
    const page = fresh();
    run(page);
    // A button whose press opens nothing.
    const tick = () => {
      page.formOpen = false;
    };
    for (let i = 0; i < 60; i += 1) {
      vi.advanceTimersByTime(500);
      tick();
    }
    expect(page.presses).toBe(20);
  });
});
