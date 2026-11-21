// SN-284 source pins for the pin surfaces: which mounts register as which id.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (p: string) => readFileSync(resolve(__dirname, '..', '..', '..', p), 'utf8');

describe('pin surface ids', () => {
  it('the blended pane registers only from the MultiChat host, never the click-through overlay', () => {
    expect(read('src/components/multichat/MultiChatWindow.tsx')).toMatch(/<BlendedChatPane channels=\{channels\} pins \/>/);
    expect(read('src/components/multichat/ChatOverlayWindow.tsx')).not.toMatch(/<BlendedChatPane[^>]*\bpins\b/);
    const blended = read('src/components/multichat/BlendedChatPane.tsx');
    expect(blended).toContain('surfaceId="blended"');
    expect(blended).toMatch(/pins && !readOnly/);
  });

  it('the chats name their surfaces', () => {
    expect(read('src/components/chat/ChatDockPane.tsx')).toContain('surfaceId="dock"');
    const widget = read('src/components/ChatWidget.tsx');
    for (const id of ["'main-provider'", "'main-chat'", '`pane:']) expect(widget).toContain(id);
    expect(read('src/mobile/screens/WatchScreen.tsx')).toContain("surfaceId: 'phone-watch'");
    expect(read('src/mobile/screens/ChatsScreen.tsx')).toContain("surfaceId: 'phone-chats'");
  });

  it('only the pins block draws a pin, and no caller still sends the v1 dismiss', () => {
    const files = [
      'src/components/mentions/MentionPins.tsx',
      'src/components/mentions/DesktopPins.tsx',
      'src/mobile/chat/PhonePins.tsx',
    ];
    for (const f of files) expect(read(f)).not.toContain("'dismiss_mention_pin'");
    expect(read('src/components/mentions/MentionPins.tsx')).toContain("'dismiss_mention_pins'");
  });

  it('the phone reports interactive only after the invoke succeeded', () => {
    const life = read('src/mobile/lifecycle.ts');
    expect(life).toMatch(/\.then\(\(\) => \{\s*lastInteractive = interactive;/);
    expect(life).toContain('export function retryInteractiveReport');
  });
});
