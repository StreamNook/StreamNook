// Every chat list hands its glide to the bottom to its own parent through
// `scrollToBottomRef`. A window-wide slot (`window.__chatScrollToBottom`) used
// to belong to whichever list mounted last, so in a window with several lists
// (MultiChat tabs and splits, docked chats) Resume glided a hidden list while
// the visible one snapped. A DOM test cannot run here (the suite has no DOM),
// so this guards the wiring in the sources.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (rel: string) => readFileSync(resolve(__dirname, '..', rel), 'utf8');

const HOSTS = [
  'components/ChatWidget.tsx',
  'components/multichat/BlendedChatPane.tsx',
  'mobile/chat/MobileChatPane.tsx',
];

describe('chat list glide wiring', () => {
  it('no list or host uses the window-wide slot', () => {
    for (const rel of ['components/ChatMessageList.tsx', ...HOSTS]) {
      expect(read(rel), rel).not.toContain('__chatScrollToBottom');
    }
  });

  it('every host passes its own ref to each list it renders', () => {
    for (const rel of HOSTS) {
      const src = read(rel);
      const lists = src.split('<ChatMessageList').slice(1);
      expect(lists.length, rel).toBeGreaterThan(0);
      for (const tail of lists) {
        const props = tail.slice(0, tail.indexOf('/>'));
        expect(props, rel).toContain('scrollToBottomRef=');
      }
    }
  });
});
