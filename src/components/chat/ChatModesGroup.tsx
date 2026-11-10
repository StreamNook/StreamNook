// The Chat modes card: the room's modes as plain switches, a length on the
// row while Followers-only or Slow mode is on, and Shield Mode, which asks in
// place before it turns on. Shared by the Tools menu, the Manage channel
// window and the phone's sheet. Every switch is a write through Rust's door
// (`chat_mode`, `shield`); a refusal shows on its row in Rust's words and the
// switch stays where Twitch has it.
import { useId, useState, type ReactNode } from 'react';
import { Clock, Hash, MessageSquare, Shield, ShieldAlert, Users } from 'lucide-react';
import { Toggle } from '../ui/Toggle';
import { managerAct, type ChatView, type ManagerTarget } from '../../services/streamManager';
import {
  chatModeAction,
  readTool,
  runTool,
  shownMode,
  toolKey,
  useManagerStore,
  type ChatModeKind,
  type LiveModes,
  type ModeState,
  type ModeWanted,
} from './managerStore';
import { GroupCard, MenuRow, RowButton, ValuePicker, type Layout } from './managerKit';

const ICON: Record<ChatModeKind, ReactNode> = {
  followers: <Users size={14} />,
  slow: <Clock size={14} />,
  subs: <Shield size={14} />,
  emotes: <MessageSquare size={14} />,
  unique: <Hash size={14} />,
};

/** A length the list does not offer (set on twitch.tv), in words. */
const lengthWords = (kind: ChatModeKind, v: number) => (kind === 'slow' ? `${v}s` : v === 0 ? 'Any follower' : `${v} min`);

export function ChatModesGroup({
  layout,
  target,
  chat,
  live,
  hideTitle = false,
}: {
  layout: Layout | 'menu';
  /** The channel the switches write to, as the account the view was read as. */
  target: ManagerTarget;
  chat: ChatView;
  /** What the room has now: IRC ROOMSTATE on the desktop, Rust's view on the phone. */
  live: LiveModes;
  /** The window's Chat modes page already says what this is. */
  hideTitle?: boolean;
}) {
  const v = useManagerStore();
  const uid = useId();
  const [confirmShield, setConfirmShield] = useState(false);
  const icons = layout !== 'sheet';
  const channelId = target.channel_id;

  const send = (mode: ChatModeKind, on: boolean, value?: number | null) => {
    const key = toolKey(channelId, `chat:${mode}`);
    const wanted: ModeWanted = { on, value: on ? (value ?? null) : null, roomAtPress: live[mode] };
    void runTool(key, () => managerAct(target, chatModeAction(mode, on, value)), { draft: wanted });
  };

  const shieldKey = toolKey(channelId, 'shield');
  const shieldEntry = readTool(v, shieldKey);
  const shieldWanted = shieldEntry.draft as { on: boolean } | undefined;
  const shieldOn =
    chat.shield === null
      ? null
      : shieldEntry.phase === 'working' && shieldWanted
        ? shieldWanted.on
        : chat.shield;
  const sendShield = (on: boolean) => {
    void runTool(shieldKey, () => managerAct(target, { kind: 'shield', on }), { draft: { on } });
  };

  return (
    <GroupCard title="Chat modes" layout={layout} hideTitle={hideTitle}>
      {chat.modes.map((m) => {
        const kind = m.kind as ChatModeKind;
        const key = toolKey(channelId, `chat:${kind}`);
        const entry = readTool(v, key);
        // Rust's read after the last write, which the act answered with.
        const fromView: ModeState = { on: m.on, value: m.on ? m.value : null };
        const shown = shownMode(entry, live[kind], fromView);
        const id = `${uid}-${kind}`;
        const value = shown.value ?? m.value ?? m.options[0]?.value ?? 0;
        const listed = m.options.some((o) => o.value === value);
        return (
          <MenuRow
            key={kind}
            layout={layout}
            icon={icons ? ICON[kind] : undefined}
            label={m.label}
            labelFor={id}
            line={entry.phase === 'failed' ? entry.line : null}
            failed
            trailing={
              <>
                {shown.on && m.options.length > 0 && (
                  <ValuePicker
                    value={value}
                    options={m.options}
                    shown={listed ? null : lengthWords(kind, value)}
                    ariaLabel={`${m.label} length`}
                    onChange={(n) => {
                      if (entry.phase !== 'working') send(kind, true, n);
                    }}
                  />
                )}
                <Toggle
                  id={id}
                  enabled={shown.on}
                  ariaLabel={m.label}
                  onChange={() => {
                    if (entry.phase !== 'working') send(kind, !shown.on);
                  }}
                />
              </>
            }
          />
        );
      })}
      {shieldOn !== null &&
        (confirmShield ? (
          <div
            onKeyDown={(e) => {
              if (e.key !== 'Escape') return;
              e.preventDefault();
              e.stopPropagation();
              setConfirmShield(false);
            }}
          >
            <MenuRow
              layout={layout}
              icon={icons ? <ShieldAlert size={14} /> : undefined}
              label="Turn on Shield Mode?"
              detail="It can clear chat and tighten every mode."
              wrapDetail
              below={
                <>
                  <RowButton
                    layout={layout}
                    tone="commit"
                    autoFocus
                    onClick={() => {
                      setConfirmShield(false);
                      sendShield(true);
                    }}
                  >
                    Turn on
                  </RowButton>
                  <RowButton layout={layout} onClick={() => setConfirmShield(false)}>
                    Cancel
                  </RowButton>
                </>
              }
            />
          </div>
        ) : (
          <MenuRow
            layout={layout}
            icon={icons ? <ShieldAlert size={14} /> : undefined}
            label="Shield Mode"
            labelFor={`${uid}-shield`}
            line={shieldEntry.phase === 'failed' ? shieldEntry.line : null}
            failed
            trailing={
              <Toggle
                id={`${uid}-shield`}
                enabled={shieldOn}
                ariaLabel="Shield Mode"
                onChange={() => {
                  if (shieldEntry.phase === 'working') return;
                  if (shieldOn) sendShield(false);
                  else setConfirmShield(true);
                }}
              />
            }
          />
        ))}
    </GroupCard>
  );
}

export default ChatModesGroup;
