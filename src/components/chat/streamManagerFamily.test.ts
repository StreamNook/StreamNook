import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// The stream manager family shows every result in place: on the button, on
// the row, in the panel. Nothing in it toasts. Its surfaces answer their own
// keys, and a list inside one closes on Esc without closing the surface.

const read = (p: string) => readFileSync(resolve(__dirname, '..', '..', p), 'utf8');

const FAMILY = [
  'components/chat/StreamManagerPanel.tsx',
  'components/chat/PollPredictionComposer.tsx',
  'components/chat/ChannelToolsMenu.tsx',
  'components/chat/ManageChannelWindow.tsx',
  'components/chat/ChatModesGroup.tsx',
  'components/chat/ModerationGroup.tsx',
  'components/chat/managerKit.tsx',
  'components/chat/UnbanRequestsPage.tsx',
  'mobile/chat/ManageChannelSheet.tsx',
];

describe('stream manager results show in place', () => {
  it.each(FAMILY)('%s never toasts', (file) => {
    expect(read(file)).not.toContain('addToast');
  });

  it("the Drops row's start and stop answer the row, not a toast", () => {
    const widget = read('components/ChatWidget.tsx');
    const start = widget.indexOf('const handleToggleAutomation');
    const end = widget.indexOf('const toolSections', start);
    expect(start).toBeGreaterThan(0);
    expect(end).toBeGreaterThan(start);
    const body = widget.slice(start, end);
    expect(body).not.toContain('addToast');
    expect(body).toContain('return { ok: true');
  });
});

describe('stream manager writes', () => {
  it("chat modes go through Rust's door, never straight to Helix", () => {
    const modes = read('components/chat/ChatModesGroup.tsx');
    expect(modes).toMatch(/managerAct\(channelId, channelLogin, chatModeAction\(/);
    for (const file of FAMILY) expect(read(file), file).not.toContain('update_chat_settings');
  });

  it('a channel switch or a lost role closes every surface, by the tested rule', () => {
    const widget = read('components/ChatWidget.tsx');
    const at = widget.indexOf('mustCloseSurfaces(');
    expect(at).toBeGreaterThan(0);
    const block = widget.slice(at, at + 600);
    for (const set of ['setManagerPanel(null)', 'setComposerKind(null)', 'setManagerWindow(null)']) expect(block).toContain(set);
  });
});

describe('stream manager keys', () => {
  it.each([
    'components/chat/managerKit.tsx',
    'components/chat/ChannelToolsMenu.tsx',
    'components/chat/ManageChannelWindow.tsx',
    'components/chat/TitleEmojiPicker.tsx',
    'components/ui/Dropdown.tsx',
  ])(
    '%s answers its own keys',
    (file) => {
      expect(read(file)).toContain('data-sn-owns-keys');
    },
  );

  it('a dropdown hears Esc first and keeps it from the surface around it', () => {
    const src = read('components/ui/Dropdown.tsx');
    expect(src).toContain("document.addEventListener('keydown', onKey, true)");
    expect(src).toMatch(/key !== 'Escape'\) return;\s*e\.preventDefault\(\);\s*e\.stopPropagation\(\);/);
  });
});
