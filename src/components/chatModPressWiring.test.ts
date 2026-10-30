// A moderator's chat never pauses by itself, and a mod button acts only once
// the rows under the pointer have held still (ChatMessageList, chat/modPress).
// The suite has no DOM, so these guard the wiring in the sources; the browser
// behavior is proven by the coordinator's rig.
import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (rel: string) => readFileSync(resolve(__dirname, '..', rel), 'utf8');
const list = read('components/ChatMessageList.tsx');

describe('no hover pause', () => {
  it('nothing holds the list on hover', () => {
    expect(list).not.toMatch(/addEventListener\('pointerover'/);
    expect(list).not.toContain('freezeRef.current.hover');
    expect(list).toContain('useRef({ press: false })');
  });

  it('the hover hold and its pill are gone', () => {
    for (const rel of ['components/chat/hoverHold.ts', 'components/chat/HoverHoldPill.tsx']) {
      expect(existsSync(resolve(__dirname, '..', rel)), rel).toBe(false);
    }
    for (const rel of ['components/ChatMessageList.tsx', 'components/ChatWidget.tsx', 'styles/globals.css']) {
      const src = read(rel);
      expect(src, rel).not.toMatch(/hoverHold|HoverHoldPill|data-hold-/);
    }
  });
});

describe('the scrollbar pauses', () => {
  it('a press on the scroller itself is a deliberate scroll-up', () => {
    expect(list).toContain('onPointerDown={handleScrollbarPress}');
    expect(list).toMatch(/e\.target !== e\.currentTarget\) return;/);
    expect(list).toContain("onPauseIntent?.({ deliberate: true });");
  });

  it('every host takes a deliberate pause past its settle windows', () => {
    expect(read('components/ChatWidget.tsx')).toMatch(/if \(opts\?\.deliberate\) \{\s*setChatPaused\(true, \{ force: true \}\);/);
    expect(read('components/multichat/BlendedChatPane.tsx')).toMatch(/if \(opts\?\.deliberate\) setChatPaused\(true, \{ force: true \}\);/);
  });
});

describe('mod buttons act only on still rows', () => {
  it('the press listener samples first and stops a press on moving rows', () => {
    const press = list.slice(list.indexOf('const onPress = (e: PointerEvent) =>'), list.indexOf('const onClick = (e: MouseEvent) =>'));
    expect(press).toContain('sample();');
    expect(press).toMatch(/if \(!isStill\(movedAt, performance\.now\(\)\) \|\| \(row instanceof HTMLElement && row\.dataset\.rowsMoving\)\) \{/);
    expect(press).toContain('e.stopPropagation();');
    expect(press).toContain('holdChatRow(control as HTMLElement, true);');
    expect(list).toContain("el.addEventListener('pointerdown', onPress, true);");
    expect(list).toContain("el.addEventListener('click', onClick, true);");
  });

  it('samples after each paint, and dims the row at the pointer only', () => {
    expect(list).toMatch(/frame = requestAnimationFrame\(\(\) => \{\s*after = window\.setTimeout\(\(\) => \{\s*sample\(\);/);
    expect(list).toContain("dimmed.dataset.rowsMoving = 'true';");
    expect(list).not.toMatch(/el\.dataset\.rowsMoving/);
  });

  it('compares rows by edges, never mod buttons by node', () => {
    expect(list).toContain('edgesMoved(was, view.get(row) ?? edgesOf(row))');
    expect(list).toContain('const again = tapAgain(tapDown, lastTap);');
  });

  it('the dimmed look rests on the moving row', () => {
    const css = read('styles/globals.css');
    expect(css).toMatch(/\[data-line-aim\] \.chat-message-row\[data-rows-moving\] :is\(\[data-row-dock\] button, \.sn-mod-line\),\s*\[data-line-aim\] \.chat-message-row\[data-rows-moving\] \[data-mod-act\] \{\s*opacity: 0\.28;\s*filter: grayscale\(1\);\s*cursor: default;\s*transition: none;/);
    // A held message's filled Allow and Deny would read as missing at 0.28.
    expect(css).toMatch(/\[data-line-aim\] \.chat-message-row\[data-rows-moving\] \[data-mod-act\] \{\s*opacity: 0\.45;\s*\}/);
  });

  it('the hover look follows the pointer at once while rows slide', () => {
    const css = read('styles/globals.css');
    expect(css).toContain('[data-line-aim] .chat-message-row:not(:hover) > .group,');
    expect(css).toContain('[data-line-aim] .chat-message-row:not(:hover) [data-row-dock] {');
  });

  it('dock tooltips open only on a real move, except the armed Ban swap', () => {
    const dock = read('components/chat/HoverModButtons.tsx');
    expect(dock).toContain('requirePointerMove={!banArmed}');
    expect(dock.match(/side="top" requirePointerMove>/g)?.length).toBe(3);
    const row = read('components/ChatMessage.tsx');
    expect(row).toContain("'Pin message'} side=\"top\" requirePointerMove>");
    expect(row).toContain('<Tooltip content="Copy message" side="top" requirePointerMove>');
  });

  it('the dock stays opaque: its buttons dim, and its box shows at once on a moving row', () => {
    const css = read('styles/globals.css');
    expect(css).toMatch(/\[data-line-aim\] \.chat-message-row\[data-rows-moving\]:hover \[data-row-dock\],[^{]*\{\s*transition: none;\s*\}/);
    expect(css).not.toMatch(/\[data-rows-moving\][^{]*\[data-row-dock\]\s*[,{][^}]*opacity/);
  });

  it('remembers every row on screen, so a jump of several rows in one frame is seen', () => {
    expect(list).toContain('function rowsInView(rows: HTMLCollection, top: number, bottom: number)');
    expect(list).toContain('const view = contentRef.current ? rowsInView(contentRef.current.children, box.top, box.bottom)');
    expect(list).toContain('moved = was === undefined || edgesMoved(was, view.get(row) ?? edgesOf(row));');
    expect(list).toContain('seen = view;');
  });
});

describe('a press holds its row', () => {
  it('the thaw waits for the click, and focus loss releases', () => {
    expect(list).toMatch(/later = window\.setTimeout\(release, 0\);/);
    expect(list).toContain("window.addEventListener('blur', release);");
    expect(list).toContain("document.addEventListener('visibilitychange', onHidden);");
  });
});

describe('the reason picker pauses chat', () => {
  it('the dock tells the list when the picker opens and closes', () => {
    const dock = read('components/chat/HoverModButtons.tsx');
    expect(dock).toContain('pickerOpen(row, true);');
    expect(dock).toContain('return () => pickerOpen(row, false);');
    expect(dock.match(/data-mod-reason="true"/g)?.length).toBe(3);
  });

  it('the list pauses deliberately and resumes only its own pause', () => {
    expect(list).toContain('el.addEventListener(PICKER_EVENT, onPicker);');
    expect(list).toContain("if (e.altKey && control.matches('[data-mod-reason]')) pauseForPicker();");
    expect(list).toContain('if (pickerPause !== null && pickerPause === userPausesRef.current) onResumeIntentRef.current?.();');
  });

  it('ChatWidget hands the list a stable resume', () => {
    const widget = read('components/ChatWidget.tsx');
    expect(widget).toContain('const handleResumeIntent = useCallback(() => {');
    expect(widget).toContain('onResumeIntent={handleResumeIntent}');
    expect(widget).toContain('onResumeIntent={onResumeIntent}');
  });
});

describe('rows that slide under a still pointer stay as they are', () => {
  it('the dock and slider mount on a real move or a press', () => {
    const row = read('components/ChatMessage.tsx');
    expect(row).toContain('onPointerMove={hoverArmed ? undefined : () => setHoverArmed(true)}');
    expect(row).toContain('onPointerDown={hoverArmed ? undefined : () => setHoverArmed(true)}');
    expect(row).not.toMatch(/onPointerEnter=\{\(e\) => \{\s*if \(!hoverArmed\)/);
  });

  it('On-hover line buttons open on the row a move is over, not on :hover', () => {
    const css = read('styles/globals.css');
    expect(css).toContain("[data-line-show='hover'][data-line-aim] .chat-message-row:not([data-line-open]) .sn-mod-line");
    expect(css).toContain("[data-line-show='hover']:not([data-line-aim]) .chat-message-row:not(:hover) .sn-mod-line");
    expect(list).toContain("el.dataset.lineAim = 'true';");
    expect(list).toContain("if (open) open.dataset.lineOpen = 'true';");
  });

  it('every row keeps its remembered height', () => {
    expect(list).toContain('containIntrinsicBlockSize: intrinsicSizeCSS,');
    expect(list).not.toContain('containIntrinsicBlockSize: paintAlways ? undefined');
  });
});
