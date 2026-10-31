// Source pins for the row swipe's wiring (the rules themselves are tested in replySwipe.test.ts and
// SwipeModerator.test.ts; these prove the call sites use them).
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const read = (p: string) => readFileSync(new URL(p, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const swipe = read('./SwipeModerator.tsx');
const pane = read('./MobileChatPane.tsx');
const input = read('./MobileChatInput.tsx');

describe('the row swipe owner', () => {
  it('holds every press, and leaves a plain lift to the list', () => {
    expect(swipe).toContain('// Hold the list now, on every surface');
    expect(swipe).toContain('holdChatRow(row, true);');
    expect(swipe).toContain('} else if (!apply) {');
  });

  it('lets an engaged swipe go at once and eats its stray click until the next press', () => {
    expect(swipe).toContain('if (apply) armSwallow();');
    expect(swipe).toContain("window.addEventListener('pointerdown', onAnyDown, true);");
  });

  it('decides left and right once, through slideStart with options, and gives up the hold menu', () => {
    expect(swipe).toContain('const move = slideStart(dx, dy, press.slider?.engage ?? REPLY_ENGAGE, press.grab, {');
    expect(swipe).toContain('live.current.cancelHold();');
  });

  it('never lets a swipe pause chat', () => {
    expect(swipe).toMatch(/const onTouchMove = \(e: TouchEvent\) => \{\s*if \(!kind\) return;\s*e\.preventDefault\(\);[^}]*e\.stopPropagation\(\);/);
    expect(swipe).toContain('pauseRatioRef.current = () => {');
  });
});

describe('the pane', () => {
  it('replies by swipe, never by a tap on a paused row', () => {
    expect(pane).not.toContain('onClick={onListClick}');
    expect(pane).not.toContain('data-tap-reply');
    expect(pane).toContain('replyFor={replyFor}');
    expect(pane).toContain('onReply={onSwipeReply}');
  });

  it('focuses the box inside the touch, and a send clears only its own reply', () => {
    expect(pane).toContain('focusRef={composerFocusRef}');
    expect(pane).toContain('onReplySent={handleReplySent}');
    expect(input).toContain('focusRef.current = focus;');
    expect(input).toContain('if (onReplySent) onReplySent(sentWith);');
  });

  it('hands the list its pause ratio only where rows take swipes', () => {
    expect(pane).toContain('touchPauseRatio={rowSwipeOn ? touchPauseRatio : undefined}');
  });

  it('lets the hold menu decide Reply and the mod tiles once, when it arms', () => {
    expect(pane).toContain('setFanArm({ canReply: rules.canReply(messageId), reach: rules.reach(messageId) });');
    expect(pane).toContain("const noReply: FanAction[] = !noSend && fanTarget && !fanArm.canReply ? ['reply'] : [];");
    expect(pane).toContain("const reach = fanTarget ? fanArm.reach : 'all';");
    expect(pane).not.toContain('replyFor(fanTarget.messageId)');
    expect(pane).toContain('const row = replyRowOf(messageId, parsed, provider, activeProvider);');
  });

  it('logs a lost reply swipe too', () => {
    expect(swipe).toContain("if (press?.target || kind === 'reply') trace('the browser took the touch (pointercancel)');");
  });
});
