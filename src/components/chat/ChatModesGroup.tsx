// The Chat modes card: the room's modes as plain switches, a length on the
// row while Followers-only or Slow mode is on, and Shield Mode, which asks in
// place before it turns on. Shared by the Tools menu, the Manage channel
// window and the phone's sheet. Every switch is a write through Rust's door
// (`chat_mode`, `shield`); a refusal shows on its row in Rust's words and the
// switch stays where Twitch has it.
import { useId, useState, type ReactNode } from 'react';
import { Clock, Hash, MessageSquare, Shield, ShieldAlert, Users } from 'lucide-react';
import { Toggle } from '../ui/Toggle';
import { managerAct, type ChatView } from '../../services/streamManager';
import {
  CHAT_MODE_HOLD_MS,
  chatModeAction,
  readTool,
  runTool,
  shownMode,
  toolKey,
  useManagerStore,
  type ChatModeKind,
  type LiveModes,
} from './managerStore';
import { GroupCard, MenuRow, RowButton, ValuePicker, type Layout } from './managerKit';

const ICON: Record<ChatModeKind, ReactNode> = {
  followers: <Users size={14} />,
  slow: <Clock size={14} />,
  subs: <Shield size={14} />,
  emotes: <MessageSquare size={14} />,
  unique: <Hash size={14} />,
};

export function ChatModesGroup({
  layout,
  channelId,
  channelLogin,
  chat,
  live,
}: {
  layout: Layout | 'menu';
  channelId: string;
  channelLogin: string;
  chat: ChatView;
  /** What the room has now: IRC ROOMSTATE on the desktop, Rust's view on the phone. */
  live: LiveModes;
}) {
  const v = useManagerStore();
  const uid = useId();
  const [confirmShield, setConfirmShield] = useState(false);
  const icons = layout !== 'sheet';

  const send = (mode: ChatModeKind, on: boolean, value?: number | null) => {
    const key = toolKey(channelId, `chat:${mode}`);
    void runTool(key, () => managerAct(channelId, channelLogin, chatModeAction(mode, on, value)), {
      draft: { on, value: on ? (value ?? null) : null },
      holdMs: CHAT_MODE_HOLD_MS,
    });
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
    void runTool(shieldKey, () => managerAct(channelId, channelLogin, { kind: 'shield', on }), { draft: { on } });
  };

  return (
    <GroupCard title="Chat modes" layout={layout}>
      {chat.modes.map((m) => {
        const kind = m.kind as ChatModeKind;
        const key = toolKey(channelId, `chat:${kind}`);
        const entry = readTool(v, key);
        const shown = shownMode(entry, live[kind]);
        const id = `${uid}-${kind}`;
        const value = shown.value ?? m.value ?? m.options[0]?.value ?? 0;
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
                    shown={shown.value === m.value ? m.value_label : null}
                    ariaLabel={`${m.label} length`}
                    onChange={(v) => {
                      if (entry.phase !== 'working') send(kind, true, v);
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
              trailing={
                <>
                  <RowButton
                    layout={layout}
                    danger
                    autoFocus
                    onClick={() => {
                      setConfirmShield(false);
                      sendShield(true);
                    }}
                  >
                    Turn on
                  </RowButton>
                  <RowButton layout={layout} onClick={() => setConfirmShield(false)}>
                    No
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
