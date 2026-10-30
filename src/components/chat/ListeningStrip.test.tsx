// The listening pill shows only in the chat the answer goes to, its Cancel
// acts only while listening, the phone never draws it, the stream's sound
// level reaches the bars through their own style (never React), and the
// answer that follows is drawn over the composer for a moment.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('../../stores/AppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ emitTo: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: 'main', listen: vi.fn() }) }));

import { ListeningStrip, ListeningStripView } from './ListeningStrip';
import { SongResultView } from './SongResultCard';
import { RESULT_MS, stripCancel, stripShows } from './stripRoom';
import type { Listening } from '../../utils/songListening';
import type { SongResult } from '../../utils/songResult';

const listening = (over: Partial<Listening> = {}): Listening => ({
  id: 'l1',
  room: 'xqc',
  phase: 'listening',
  attempt: 1,
  attempts: 2,
  captureMs: 8000,
  startedAt: null,
  remote: null,
  local: true,
  ...over,
});

const read = (p: string) =>
  readFileSync(resolve(__dirname, p), 'utf8')
    // Comments may name the hooks.
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\/\/.*$/gm, '');

describe('where the pill shows', () => {
  it("only in the chat whose room the answer goes to, folded on the widget's side too", () => {
    expect(stripShows(listening(), 'xqc')).toBe(true);
    expect(stripShows(listening(), 'XQC')).toBe(true);
    expect(stripShows(listening({ room: 'kick:adinross' }), 'kick:AdinRoss')).toBe(true);
    expect(stripShows(listening(), 'kaicenat')).toBe(false);
    expect(stripShows(listening(), null)).toBe(false);
    // A listening that names no room shows in no chat.
    expect(stripShows(listening({ room: null }), 'xqc')).toBe(false);
    expect(stripShows(null, 'xqc')).toBe(false);
  });

  it('is drawn from the shared state only (its server snapshot holds only the empty anchor)', () => {
    const html = renderToStaticMarkup(<ListeningStrip room="xqc" />);
    expect(html).not.toContain('role="status"');
    expect(html).toMatch(/^<div class="pointer-events-none absolute[^"]*"><\/div>$/);
  });
});

describe('what the pill says and does', () => {
  it('listening: the bars, the words with their dots, the time line, and Cancel', () => {
    const html = renderToStaticMarkup(<ListeningStripView listening={listening()} onCancel={() => {}} />);
    expect(html).toContain('Listening for the song');
    expect(html).toContain('sn-listen-dots');
    expect(html).toContain('sn-listen-bars sn-listen-bars--accent');
    expect(html).toContain('sn-listen-time');
    // The bars sit in the 40 px slot the answer's cover takes.
    expect(html).toMatch(/class="flex h-10 w-10 shrink-0 items-center justify-center"[^>]*><span[^>]*class="sn-listen-bars/);
    expect(html).toMatch(/<button[^>]*aria-label="Cancel"[^>]*>/);
    expect(html).not.toMatch(/<button[^>]*disabled[^>]*aria-label="Cancel"/);
  });

  it('a second attempt says so', () => {
    const html = renderToStaticMarkup(<ListeningStripView listening={listening({ attempt: 2 })} onCancel={() => {}} />);
    expect(html).toContain('Trying again');
  });

  it('finding: the words change, and Cancel holds its place without acting', () => {
    const html = renderToStaticMarkup(<ListeningStripView listening={listening({ phase: 'finding' })} onCancel={null} />);
    expect(html).toContain('Finding the song');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*class="[^"]*opacity-0[^"]*"/);
  });

  it('Cancel is wired only while listening', () => {
    const cancel = vi.fn();
    expect(stripCancel(listening(), cancel)).toBe(cancel);
    expect(stripCancel(listening({ phase: 'finding' }), cancel)).toBeNull();
    expect(stripCancel(null, cancel)).toBeNull();
  });
});

describe('the answer over the composer', () => {
  const found: SongResult = {
    id: 1,
    room: 'xqc',
    result: {
      status: 'match',
      song: {
        title: 'Kiss of Life',
        artist: 'Sade',
        album: 'Love Deluxe',
        album_art: 'https://example.com/a.jpg',
        shazam_url: null,
        song_link: null,
        key: 'sade\u001fkiss of life',
        providers: [
          { name: 'Apple Music', url: 'https://music.apple.com/us/song/1' },
          { name: 'Deezer', url: 'https://www.deezer.com/track/2' },
          { name: 'YouTube', url: 'https://www.youtube.com/watch?v=abcdefghijk' },
          { name: 'YouTube Music', url: 'https://music.youtube.com/watch?v=abcdefghijk' },
          { name: 'Spotify', url: 'https://open.spotify.com/search/x', search: true },
        ],
      },
    },
  };

  it('a song: "Found it", its name on one line, at most three services with a mark of their own', () => {
    const html = renderToStaticMarkup(<SongResultView answer={found} onDone={() => {}} />);
    expect(html).toContain('Found it');
    expect(html).toContain('Kiss of Life');
    expect(html).toContain(' · Sade');
    expect(html).toContain('aria-label="Open in Apple Music"');
    expect(html).toContain('aria-label="Open in YouTube"');
    expect(html).toContain('aria-label="Open in YouTube Music"');
    // Deezer draws only the plain note, so it gets no button; searches never do.
    expect(html).not.toContain('Open in Deezer');
    expect(html).not.toContain('Spotify');
    expect(html).toContain('aria-label="Dismiss"');
    // No "Open" text button: the row is the way into Your songs.
    expect(html).not.toMatch(/>Open</);
  });

  it('a miss: a short pill with Try again; an error: its own words, no Try again', () => {
    const miss = renderToStaticMarkup(<SongResultView answer={{ id: 2, room: 'xqc', result: { status: 'no-match' } }} onDone={() => {}} />);
    expect(miss).toContain('Couldn&#x27;t hear a song');
    expect(miss).toContain('Try again');
    const error = renderToStaticMarkup(
      <SongResultView answer={{ id: 3, room: 'xqc', result: { status: 'error', message: 'No stream is playing.' } }} onDone={() => {}} />,
    );
    expect(error).toContain('No stream is playing.');
    expect(error).not.toContain('Try again');
  });

  it('a song shows longer than a miss', () => {
    expect(RESULT_MS.match).toBeGreaterThan(RESULT_MS.other);
  });

  it('floats over the chat in one shape whose contents cross-fade, and leaves toward the composer', () => {
    const src = read('ListeningStrip.tsx');
    // A zero-height anchor: nothing in the shelf moves.
    expect(src).toContain('pointer-events-none absolute inset-x-2 bottom-full z-30 mb-4');
    expect(src).toContain('pointer-events-auto w-full');
    expect(src).toContain("transformOrigin: 'bottom center'");
    expect(src).toMatch(/leave: \(off: boolean\) => \(\{ opacity: 0, y: 6, scale: 0\.98/);
    expect(src).toMatch(/key=\{contents\}/);
    // One shape: its key never changes between listening, finding and the answer.
    expect(src).toContain('key="shape"');
  });

  it('lifts off the chat on a quiet neutral shadow, lighter on a light theme', () => {
    const strip = read('stripRoom.ts');
    expect(strip).toContain("'0 6px 18px -8px rgba(0, 0, 0, 0.45)'");
    expect(strip).toContain("'0 6px 16px -8px rgba(0, 0, 0, 0.2)'");
    expect(strip).not.toMatch(/--color-accent\) 60%/);
  });
});

describe('the sound level never goes through React', () => {
  it('the pill keeps no React state, and the level is no store', () => {
    const src = read('ListeningStrip.tsx');
    expect(src).not.toMatch(/\buseState\s*[(<]/);
    expect(src).not.toMatch(/\buseReducer\s*[(<]/);
    expect(src).not.toMatch(/useSyncExternalStore\(\s*subscribeLevel/);
    expect(src).toContain('drawBars(');
  });

  it("the shared renderer writes each level to the bars' own transforms, outside React", () => {
    const bars = read('listeningBars.ts');
    expect(bars).not.toMatch(/from 'react'/);
    expect(bars).toContain('subscribeLevel(');
    expect(bars).toMatch(/bar\.style\.transform = `scaleY\(/);
    expect(bars).toContain('requestAnimationFrame(tick)');
  });

  it('the pill is the desktop’s: the phone draws none', () => {
    expect(read('ListeningStrip.tsx')).toMatch(/if \(IS_MOBILE\) return null;/);
  });
});

describe('every chat mounts it in its composer shelf', () => {
  const widget = readFileSync(resolve(__dirname, '..', 'ChatWidget.tsx'), 'utf8');

  it('with the room its slash commands answer into, above the reply bar', () => {
    const mount = widget.indexOf('<ListeningStrip room={isTwitch ? (currentStream?.user_login ?? null) : providerKey} />');
    expect(mount).toBeGreaterThan(0);
    expect(widget.indexOf('{replyingTo && !isResubMode && !isWatchStreakMode && (', mount)).toBeGreaterThan(mount);
    // Inside the shelf, which only draws on the chat view.
    const shelf = widget.lastIndexOf("{activeView === 'chat' &&", mount);
    expect(shelf).toBeGreaterThan(widget.lastIndexOf('<ChatMessagesPanel', mount));
    expect(widget.slice(shelf, mount)).toContain('border-t border-borderSubtle');
  });
});
