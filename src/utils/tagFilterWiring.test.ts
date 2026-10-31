// The tag filter's tested rules (tagFilter.test.ts) only help where the screens
// call them; these pin the calls in place (SN-78).
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8').replace(/\r\n/g, '\n');

describe('tag filter wiring', () => {
  it('the hook arms retries only through retryStep, keyed by revision, for live accepted answers', () => {
    const hook = read('src/hooks/useCategoryTagSuggestions.ts');
    expect(hook).toContain('retryStep(retry.current, revisionKey, got)');
    expect(hook).toContain('revisionId.current += 1;');
    expect(hook).toContain('if (!live || !acceptAnswer(reqId, latest.current)) return;');
    expect(hook).toContain('if (timer) clearTimeout(timer);');
  });

  it('card pills go through pillClick and the band carries the card hold', () => {
    const tags = read('src/mobile/ui/StreamCardTags.tsx');
    expect(tags).toContain('pillClick({');
    expect(tags).toContain('{...bandHandlers}');
    expect(read('src/mobile/ui/MobileStreamCard.tsx')).toContain('bandHandlers={hold.handlers}');
    expect(read('src/mobile/ui/ChannelHold.tsx')).toContain('useState(createHoldFlag)');
  });

  it('both category screens page through the shared reducer', () => {
    const phone = read('src/mobile/screens/CategoryStreamsScreen.tsx');
    for (const call of ['newTagList(', 'applyPage(', 'failPage(', 'beginMore(', 'refreshTagList(']) {
      expect(phone).toContain(call);
    }
    const desk = read('src/components/Home.tsx');
    for (const call of ['newTagList(', 'applyPage(', 'failPage(', 'beginMore(', 'categoryTagsFor(']) {
      expect(desk).toContain(call);
    }
  });

  it('PullToRefresh keeps its own ref and only copies it out', () => {
    const pull = read('src/mobile/ui/PullToRefresh.tsx');
    expect(pull).toContain('ref={scrollRef}');
    expect(pull).not.toContain('ref={scrollerRef');
    expect(pull).toContain('scrollerRef.current = scrollRef.current;');
  });
});
