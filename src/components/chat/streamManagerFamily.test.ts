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
    expect(modes).toMatch(/managerAct\(target, chatModeAction\(/);
    for (const file of FAMILY) expect(read(file), file).not.toContain('update_chat_settings');
  });

  it('every write goes as the account the form was showing', () => {
    // A write names its channel and account (managerAct(target, action)), never a bare channel.
    for (const file of FAMILY) expect(read(file), file).not.toMatch(/managerAct\(\s*(ch|channelId)\s*,/);
    expect(read('services/streamManager.ts')).toContain('expectedUserId: target.user_id');
  });

  it('Stream info saves only the fields you changed', () => {
    const panel = read('components/chat/StreamManagerPanel.tsx');
    expect(panel).toMatch(/kind: 'save_info', draft, fields \}/);
    expect(panel).not.toContain('infoBase');
  });

  it('the builder keeps no recents of its own: Rust does', () => {
    const builder = read('components/chat/PollPredictionComposer.tsx');
    expect(builder).toContain('view.recent_polls');
    expect(builder).not.toContain('updateSettings');
  });

  it('a channel switch or a lost role closes every surface, by the tested rule', () => {
    const widget = read('components/ChatWidget.tsx');
    const at = widget.indexOf('mustCloseSurfaces(');
    expect(at).toBeGreaterThan(0);
    const block = widget.slice(at, at + 600);
    for (const set of ['setManagerPanel(null)', 'setComposerKind(null)', 'setManagerWindow(null)']) expect(block).toContain(set);
  });
});

describe('who manages here', () => {
  it('the chat reads the manager before it decides its Tools, so editors get them', () => {
    const widget = read('components/ChatWidget.tsx');
    const hook = widget.indexOf('useStreamManager(');
    const tools = widget.indexOf('channelToolSections(');
    expect(hook).toBeGreaterThan(0);
    expect(hook).toBeLessThan(tools);
    expect(widget.slice(hook, tools)).toContain('managesHere(');
    expect(read('mobile/chat/MobileChatInput.tsx')).toContain('managesHere(');
  });

  it('Rust keeps an open surface current: each one says so', () => {
    const widget = read('components/ChatWidget.tsx');
    for (const open of ['!!managerPanel', '!!composerKind', '!!managerWindow', 'toolsMenuOpen']) {
      expect(widget).toContain(`useManagerInterest(${open},`);
    }
    expect(read('mobile/chat/ManageChannelSheet.tsx')).toMatch(/useManagerInterest\(open, channelId, channelLogin, \{ pauseWhileHidden: true \}\)/);
  });
});

/** A source with Windows line ends made plain. */
const src = (p: string) => read(p).replace(/\r\n/g, '\n');
/** From the first `from` to the next `to` after it. */
const between = (s: string, from: string, to: string) => {
  const a = s.indexOf(from);
  expect(a, from).toBeGreaterThanOrEqual(0);
  const b = s.indexOf(to, a + from.length);
  expect(b, to).toBeGreaterThan(a);
  return s.slice(a, b);
};
/** The effect that adds the listener for `event`, up to where it adds it. */
const listenerOf = (s: string, event: string) => {
  const at = s.indexOf(`addEventListener('${event}'`);
  expect(at, event).toBeGreaterThan(0);
  return s.slice(s.lastIndexOf('useEffect(', at), at);
};

describe('the follow-up fixes stay wired', () => {
  it('a failed read is tried again on the backoff', () => {
    const retry = between(src('services/streamManager.ts'), 'const failed = active', 'const setQueue');
    expect(retry).toContain('retryDelay(failures.current)');
  });

  it('a Stream info save lands its fields, and only for the account it began as', () => {
    const save = between(src('components/chat/StreamManagerPanel.tsx'), 'const save = async () => {', 'const startOver');
    expect(save.indexOf('const epochAtSave = storeEpoch()')).toBeLessThan(save.indexOf('await runTool('));
    expect(save).toContain('if (ok) landInfoSave(key, sent, epochAtSave)');
    const land = between(src('components/chat/managerStore.ts'), 'export function landInfoSave', '\n}\n');
    for (const part of ['if (epochAtSave !== epoch) return false', 'landingAfterSave(', 'changeDraftLater(']) expect(land).toContain(part);
  });

  it("Twitch's read showing a sent field changes only the draft, never a failure's line", () => {
    const effect = between(src('components/chat/StreamManagerPanel.tsx'), '// A sent field shows until', '}, [key, info]);');
    expect(effect).toContain('replaceToolDraft(key, landedInfo(');
    expect(effect).not.toContain('setToolDraft(');
  });

  it('a Tools menu that goes away while open is not left counted as open', () => {
    expect(src('components/chat/ChannelToolsMenu.tsx')).toContain('useEffect(() => () => onOpenChange?.(false), [onOpenChange]);');
    expect(src('components/ChatWidget.tsx')).toContain('if (!toolSections.any && toolsMenuOpen) setToolsMenuOpen(false);');
  });

  it("the phone's sheet does not come back open by itself", () => {
    expect(src('mobile/chat/MobileChatInput.tsx')).toContain("if (manageOpen && manages !== 'yes') setManageOpen(false);");
  });

  it('interest names and numbers survive a reload', () => {
    const sm = src('services/streamManager.ts');
    expect(sm).toContain('let watchSeq = firstWatchSeq(PAGE_START);');
    expect(sm).toContain('useState(() => surfaceId(PAGE_START, (surfaceCount += 1)))');
  });
});

describe('slash commands reach only their pane', () => {
  it.each([
    ['components/ChatWidget.tsx', 'streamnook-open-poll-composer', 'acceptsComposerEvent(detail, paneSource, currentStream?.user_login)'],
    ['components/ChatWidget.tsx', 'streamnook-clear-local-chat', 'acceptsComposerEvent((e as CustomEvent<PaneEventDetail>).detail, paneSource, currentStream?.user_login)'],
    ['mobile/chat/MobileChatInput.tsx', 'streamnook-open-poll-composer', 'acceptsComposerEvent(detail, source, channel)'],
    ['mobile/chat/MobileChatPane.tsx', 'streamnook-clear-local-chat', 'acceptsComposerEvent((e as CustomEvent<PaneEventDetail>).detail, paneSource, null)'],
  ])('%s: the %s listener asks acceptsComposerEvent', (file, event, call) => {
    expect(listenerOf(src(file), event)).toContain(call);
  });

  it('the pane events carry their source', () => {
    const ui = read('utils/slashUi.ts');
    expect(ui).toContain("'streamnook-clear-local-chat', { detail: { source, channel } }");
    expect(ui).toMatch(/'streamnook-open-poll-composer', \{ detail: \{ source,/);
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
