import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (p: string) => readFileSync(resolve(__dirname, p), 'latin1').replace(/\r\n/g, '\n');

describe('the reply card over the composer', () => {
  it('draws who you reply to the way their chat row does, through the pinned message\'s chatter look', () => {
    const card = read('ReplyContextCard.tsx');
    expect(card).toContain('<PinnedChatter');
    expect(card).toContain('variant="line"');
    expect(card).toContain('shownName={row.metadata?.shown_name}');
    expect(card).toContain('${GLAZE_BANNER} relative mb-2');
    // Motion Off and Performance mode show it at once.
    expect(card).toContain('instant ? { duration: 0 }');
  });

  it('is fed the replied-to row when a reply starts, and replaces the old plain strip', () => {
    const widget = read('../ChatWidget.tsx');
    expect(widget).toContain("const row = messagesRef.current.find((m) => typeof m !== 'string' && m.id === messageId);");
    expect(widget).toContain('<ReplyContextCard');
    expect(widget).not.toContain('Replying to <span className="text-accent font-semibold">');
  });
});
