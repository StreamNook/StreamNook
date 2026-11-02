// A tap on the "+N" pill must not also open its row's line buttons. The list's
// tap handler skips anything inside `[data-no-row-tap]`, and the pill carries it.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

describe('row tap skips the fold chip', () => {
  it('the list tap handler returns early inside [data-no-row-tap]', () => {
    const src = read('src/components/ChatMessageList.tsx');
    const at = src.indexOf('const onTap = (e: MouseEvent) => {');
    expect(at).toBeGreaterThan(-1);
    const body = src.slice(at, src.indexOf('};', at));
    expect(body).toContain("closest('[data-no-row-tap]')");
  });

  it('the chip carries the marker', () => {
    expect(read('src/components/chat/BadgeFold.tsx')).toContain('data-no-row-tap');
  });
});
