import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const read = (p: string) => readFileSync(new URL(p, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const message = read('./ChatMessage.tsx');
const list = read('./ChatMessageList.tsx');
const chip = read('./chat/CreatorChip.tsx');
const pane = read('../mobile/chat/MobileChatPane.tsx');
const watch = read('../mobile/screens/WatchScreen.tsx');
const sheet = read('../mobile/ui/MobileSheet.tsx');

describe('an emote tap in a chat row', () => {
  it('reads the one provider rule and routes through the phone opener first', () => {
    expect(message).toContain('const source = emoteRef(emoteUrl, segment.emoteId);');
    expect(message).toContain('const targets = stackTargets(stack ?? [segment], creditRoom);');
    expect(message).toContain('emoteClickRoute(source, useEmoteCardStore.getState().seventvSpotlight)');
  });

  it("hands every image of a stack the stack's members", () => {
    expect(message).toContain('              stackMembers,\n');
  });

  it('threads the opener through the list to every row', () => {
    expect(list.match(/onEmoteOpen=\{onEmoteOpen\}/g)?.length).toBe(2);
  });

  it('the phone passes one opener unconditionally, and hands off to the profile once the sheet has gone', () => {
    expect(pane).toContain('            onEmoteOpen={onEmoteOpen}\n');
    expect(pane).toContain('onExited={onEmoteExited}');
    expect(pane).toContain('    setEmoteOpen(false);\n    pendingProfile.current = null;\n  }, [paused, closeFan]);');
    expect(watch).toContain("hidden={chatClass === 'hidden'}");
    expect(sheet).toContain('<AnimatePresence onExitComplete={onExited}>');
  });

  it("a creator chip's click goes through its route", () => {
    expect(chip).toContain('const route = creatorRoute(creator, !!onOpenTwitch);');
    expect(chip).toContain('openCreator(creator, onLeave, onOpenTwitch)');
  });
});
