// @vitest-environment happy-dom
// A whisper draft survives its thread closing: a link closes every layer
// (closeLinkSheet), and the composer's words come back when the conversation
// opens again in the real host (You > Inbox) driven by whisperKey.
import type { ReactNode } from 'react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const fake = vi.hoisted(() => ({
  owner: { id: 'A' },
  sends: [] as Array<{ text: string; finish: (err: string | null) => void }>,
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../../stores/AppStore', () => ({
  useAppStore: {
    getState: () => ({ startStream: vi.fn(), currentUser: { user_id: fake.owner.id }, streamUrl: null, isLoading: false }),
    subscribe: () => () => {},
  },
}));
vi.mock('../../stores/badgeSpotlightStore', () => ({
  useBadgeSpotlightStore: { getState: () => ({ close: () => {} }) },
}));
vi.mock('../phonePrefs', () => ({ startTab: () => 'following' }));
vi.mock('../../utils/logger', () => ({ Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));
vi.mock('../ui/DrillInScreen', () => ({
  DrillInScreen: ({ open, children }: { open: boolean; children: ReactNode }) => (open ? <div>{children}</div> : null),
}));
vi.mock('../mentions/InboxPanel', () => ({ InboxPanel: () => <div data-testid="inbox-list" /> }));
vi.mock('../mentions/mentionInbox', () => ({ closeInboxSheet: vi.fn() }));
vi.mock('../mentions/InboxParts', () => ({ InboxParts: ({ text }: { text: string }) => <span>{text}</span> }));
vi.mock('../../components/SevenTvAvatar', () => ({ SevenTvAvatar: () => <span /> }));
vi.mock('../../components/chat/PinnedChatter', () => ({ PinnedChatter: () => <span /> }));
vi.mock('../../components/whispers/whisperView', () => ({ liveLine: () => null }));
vi.mock('./whisperInbox', async () => {
  const { create } = await import('zustand');
  const meta = (key: string) => ({
    key,
    user_id: '9',
    user_login: 'friend',
    user_name: 'Friend',
    profile_image_url: null,
    color: null,
    badges: [],
    live: null,
  });
  return {
    useWhispersStore: create(() => ({ status: null, revision: 0, lastUpdate: null })),
    loadWhisperPage: vi.fn(async (key: string) => ({
      meta: meta(key),
      messages: [],
      cursor: null,
      has_more_local: false,
      older_on_twitch: false,
    })),
    loadOlderFromTwitch: vi.fn(async () => 0),
    sendWhisper: vi.fn(
      (_meta: unknown, text: string) =>
        new Promise<string | null>((finish) => {
          fake.sends.push({ text, finish });
        }),
    ),
    retryWhisper: vi.fn(),
    discardWhisper: vi.fn(),
    archiveWhisper: vi.fn(async () => {}),
    setOpenWhisper: vi.fn(),
    whisperDraftOwner: () => fake.owner.id,
  };
});

import { MentionsScreen } from '../screens/MentionsScreen';
import { useMobileNavStore } from '../navStore';
import { forwardWhisperDraft, resetWhisperDrafts, whisperDraftKey } from './whisperModel';

let root: Root;
let host: HTMLDivElement;

const flush = () =>
  act(async () => {
    await new Promise((done) => setTimeout(done, 0));
  });
const composer = () => document.querySelector('textarea');

async function openThread(key: string) {
  await act(async () => {
    useMobileNavStore.setState({ mentionsOpen: true, whisperKey: key });
  });
  await flush();
}

async function closeByLink() {
  await act(async () => {
    useMobileNavStore.getState().closeLinkSheet();
  });
  await flush();
}

async function type(text: string) {
  const area = composer();
  if (!area) throw new Error('no composer');
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
    setValue?.call(area, text);
    area.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function pressSend() {
  await act(async () => {
    document.querySelector<HTMLButtonElement>('button[aria-label="Send"]')?.click();
  });
}

async function finishSend(err: string | null) {
  await act(async () => {
    fake.sends.shift()?.finish(err);
  });
  await flush();
}

beforeEach(async () => {
  resetWhisperDrafts();
  fake.sends.length = 0;
  fake.owner.id = 'A';
  useMobileNavStore.setState({ mentionsOpen: false, whisperKey: null, sheetStack: [] });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => root.render(<MentionsScreen />));
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

describe('whisper drafts in the thread', () => {
  // Fails while the composer starts empty on every mount (the words are lost to the link).
  it('come back after a link closes the thread', async () => {
    await openThread('T');
    await type('hello');
    await closeByLink();
    expect(composer()).toBeNull();
    await openThread('T');
    expect(composer()?.value).toBe('hello');
  });

  // Fails if a refused send's words are lost because the thread was closed when the refusal came.
  it('bring back a refused send that finished while the thread was closed', async () => {
    await openThread('T');
    await type('hi there');
    await pressSend();
    expect(fake.sends.map((s) => s.text)).toEqual(['hi there']);
    await closeByLink();
    await finishSend('Refused');
    await openThread('T');
    expect(composer()?.value).toBe('hi there');
  });

  // Fails if a reopen mid-send brings back words that went out.
  it('stay empty after a send that went out while the thread was closed', async () => {
    await openThread('T');
    await type('hi there');
    await pressSend();
    await closeByLink();
    await finishSend(null);
    await openThread('T');
    expect(composer()?.value).toBe('');
  });

  // Fails if a refusal overwrites what was typed during the send.
  it('keep text typed during a send that is then refused', async () => {
    await openThread('T');
    await type('first');
    await pressSend();
    await type('second');
    await finishSend('Refused');
    expect(composer()?.value).toBe('second');
    await closeByLink();
    await openThread('T');
    expect(composer()?.value).toBe('second');
  });

  // Fails if a draft typed under one account shows under another.
  it('belong to the account that typed them', async () => {
    await openThread('T');
    await type('hello');
    await closeByLink();
    fake.owner.id = 'B';
    await openThread('T');
    expect(composer()?.value).toBe('');
  });

  // Fails if text typed after a re-key lands under the old key (the reopened new key shows older text).
  it('follow a re-key while the thread is open', async () => {
    await openThread('T');
    await type('one');
    forwardWhisperDraft(whisperDraftKey('A', 'T'), whisperDraftKey('A', 'K'));
    await type('one two');
    await closeByLink();
    await openThread('K');
    expect(composer()?.value).toBe('one two');
  });

  it('follow a re-key while the thread is closed', async () => {
    await openThread('T');
    await type('closed');
    await closeByLink();
    forwardWhisperDraft(whisperDraftKey('A', 'T'), whisperDraftKey('A', 'K'));
    await openThread('K');
    expect(composer()?.value).toBe('closed');
  });

  // Fails if a refusal that lands after a re-key writes under the old key.
  it('bring a refused send back under the new key after a re-key', async () => {
    await openThread('T');
    await type('hi there');
    await pressSend();
    await closeByLink();
    forwardWhisperDraft(whisperDraftKey('A', 'T'), whisperDraftKey('A', 'K'));
    await finishSend('Refused');
    await openThread('K');
    expect(composer()?.value).toBe('hi there');
  });
});
