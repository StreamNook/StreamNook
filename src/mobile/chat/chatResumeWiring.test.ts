// The return's one owner (lifecycle -> chatResume), pinned by source: the
// ordering is the rule, and nothing else should re-join on its own.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (p: string) => readFileSync(resolve(__dirname, p), 'utf8');

describe('chat return wiring', () => {
  it('lifecycle hands the return to resumeChats and keeps no rebuild of its own', () => {
    const src = read('../lifecycle.ts');
    expect(src).toContain("import('./chat/chatResume')");
    expect(src).toContain('resumeChats(awayFor)');
    expect(src).not.toContain('hardCycleChat');
  });

  it('the chats lifecycle only times the park', () => {
    expect(read('./useChatsLifecycle.ts')).not.toMatch(/\.connect\(/);
  });

  it('the Chats screen joins only while the page is seen', () => {
    const src = read('../screens/ChatsScreen.tsx');
    expect(src).toContain("shown && pageVisible && connection !== 'connected'");
    // Visibility is a dependency, so a page that opened hidden joins once seen.
    expect(src).toContain('[shown, pageVisible, connection, connect]');
  });
});
