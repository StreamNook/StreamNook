// Unsent whisper text: kept by account and conversation, followed across a
// re-key, and cleared when the account changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const hub = vi.hoisted(() => ({
  handlers: new Map<string, (e: { payload: unknown }) => void>(),
  user: { id: 'A' as string | null },
  userListeners: [] as Array<(s: unknown, prev: unknown) => void>,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    hub.handlers.set(event, handler);
    return () => {
      hub.handlers.delete(event);
    };
  }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('../../stores/AppStore', () => ({
  useAppStore: {
    getState: () => ({ currentUser: hub.user.id ? { user_id: hub.user.id } : null, addToast: vi.fn() }),
    subscribe: (listener: (s: unknown, prev: unknown) => void) => {
      hub.userListeners.push(listener);
      return () => {};
    },
  },
}));
vi.mock('../navStore', () => ({
  useMobileNavStore: { getState: () => ({ openWhisper: vi.fn(), setMentionsOpen: vi.fn(), mentionsOpen: false }) },
}));
vi.mock('../mentions/mentionInbox', () => ({
  bumpInbox: vi.fn(),
  inboxHooks: { openWhisper: null },
  useMentionsStore: { getState: () => ({ sheetIn: null }), setState: vi.fn() },
}));
vi.mock('../../utils/whisperClaim', () => ({ claimWhisper: vi.fn() }));
vi.mock('../../utils/logger', () => ({ Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));

import {
  clearWhisperDraft,
  forwardWhisperDraft,
  getWhisperDraft,
  latestDraftKey,
  resetWhisperDrafts,
  setWhisperDraft,
  whisperDraftKey,
} from './whisperModel';
import { installWhisperInbox, whisperDraftOwner } from './whisperInbox';

const T = whisperDraftKey('A', 'T');
const K = whisperDraftKey('A', 'K');

beforeEach(() => {
  resetWhisperDrafts();
  hub.user.id = 'A';
});

describe('whisper drafts', () => {
  it('keep text by account and conversation, and an empty write forgets it', () => {
    setWhisperDraft(T, 'hello');
    expect(getWhisperDraft(T)).toBe('hello');
    expect(getWhisperDraft(whisperDraftKey('B', 'T'))).toBe('');
    setWhisperDraft(T, '');
    expect(getWhisperDraft(T)).toBe('');
    setWhisperDraft(T, 'again');
    clearWhisperDraft(T);
    expect(getWhisperDraft(T)).toBe('');
  });

  // Fails if a re-key loses the text, or a write through the old key lands where nobody reads.
  it('follow a re-key: text under the old key moves, and later writes through the old key land under the new one', () => {
    setWhisperDraft(T, 'typed');
    forwardWhisperDraft(T, K);
    expect(getWhisperDraft(K)).toBe('typed');
    expect(getWhisperDraft(T)).toBe('typed');
    setWhisperDraft(T, 'typed more');
    expect(getWhisperDraft(K)).toBe('typed more');
  });

  it('never move an empty value over the new key', () => {
    setWhisperDraft(K, 'newer');
    forwardWhisperDraft(T, K);
    expect(getWhisperDraft(K)).toBe('newer');
  });

  it('stop on a loop and after eight hops', () => {
    forwardWhisperDraft('a', 'b');
    forwardWhisperDraft('b', 'a');
    expect(latestDraftKey('a')).toBe('b');
    resetWhisperDrafts();
    for (let i = 0; i < 10; i++) forwardWhisperDraft(`k${i}`, `k${i + 1}`);
    expect(latestDraftKey('k0')).toBe('k8');
  });

  it('are all forgotten by a reset, re-keys included', () => {
    setWhisperDraft(T, 'typed');
    forwardWhisperDraft(T, K);
    resetWhisperDrafts();
    expect(getWhisperDraft(K)).toBe('');
    expect(latestDraftKey(T)).toBe(T);
  });
});

describe('the inbox keeps drafts in step with Rust', () => {
  let stop: () => void = () => {};
  beforeEach(() => {
    hub.handlers.clear();
    hub.userListeners.length = 0;
    stop = installWhisperInbox();
  });
  afterEach(() => stop());

  const update = (over: Record<string, unknown>) => ({
    payload: {
      owner_id: 'A',
      key: 'K',
      replaced_key: null,
      meta: {},
      message: null,
      removed_message_id: null,
      loading: null,
      error: null,
      ...over,
    },
  });

  // Fails if only an open thread moves its draft: a closed thread's text must follow the re-key too.
  it("moves a re-keyed conversation's draft, open thread or not", () => {
    setWhisperDraft(T, 'typed');
    hub.handlers.get('whisper-conversation-updated')?.(update({ replaced_key: 'T' }));
    expect(getWhisperDraft(K)).toBe('typed');
  });

  it('leaves drafts alone for an update without a re-key', () => {
    setWhisperDraft(T, 'typed');
    hub.handlers.get('whisper-conversation-updated')?.(update({ key: 'T' }));
    expect(getWhisperDraft(T)).toBe('typed');
    expect(latestDraftKey(T)).toBe(T);
  });

  // Fails if a draft typed under one account shows under another after a switch or a sign-out.
  it('clears every draft when the account changes', () => {
    setWhisperDraft(T, 'typed');
    hub.user.id = 'B';
    for (const listener of hub.userListeners) {
      listener({ currentUser: { user_id: 'B' } }, { currentUser: { user_id: 'A' } });
    }
    expect(getWhisperDraft(T)).toBe('');
  });

  it('keys drafts by the signed-in account', () => {
    expect(whisperDraftOwner()).toBe('A');
    hub.user.id = null;
    expect(whisperDraftOwner()).toBe('');
  });
});
