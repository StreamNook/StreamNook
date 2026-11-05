// Every open puts its chat in front on the phones, the same channel again
// included (Rust `ChatTabs::open_stream`). Pinned by source: the page only
// counts the opens and says so; the rule itself is tested in chat_tabs.rs.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (p: string) => readFileSync(resolve(__dirname, p), 'utf8');

/** The body of the store action or function that starts at `head`. */
function body(src: string, head: string): string {
  const start = src.indexOf(head);
  expect(start, `${head} not found`).toBeGreaterThan(-1);
  // From the opening brace, so a parameter list (`set: (partial...`) is not
  // taken for the next action.
  const open = start + src.slice(start).search(/\{\r?\n/);
  const next = src.slice(open).search(/\n {2}\w+: (async )?\(|\nasync function |\nfunction /);
  return src.slice(start, next < 0 ? undefined : open + next);
}

describe('an open puts its chat in front', () => {
  const store = read('../../stores/AppStore.ts');

  it('every open counts, and nothing else does', () => {
    for (const head of [
      'async function startProviderStream(',
      '  startStream: async (',
      '  startOfflineChat: async (',
      '  playMedia: async (',
      '  playOfflineRoomVod: async (',
    ]) {
      expect(body(store, head), head).toMatch(/streamOpenSeq: get\(\)\.streamOpenSeq \+ 1/);
    }
    expect(store.match(/streamOpenSeq: get\(\)\.streamOpenSeq \+ 1/g)).toHaveLength(5);
  });

  it('the chats lifecycle asks for focus only when the count moved', () => {
    const src = read('./useChatsLifecycle.ts');
    expect(src).toContain('const opened = openSeq !== focusedSeq.current;');
    expect(src).toMatch(/streamProv,\s+opened,\s+\);/);
    expect(src).toMatch(/userId,\s+openSeq,\s+\]\);/);
  });
});
