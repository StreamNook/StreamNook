// The chat command browser, phone shape. Same command table, same locks and
// same search as the desktop menu; the desktop shell is a three-column
// popover built for a chat widget and does not fold into a phone sheet.
import React, { useMemo, useState } from 'react';
import { Lock } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { RAIL, lockFor, matches } from '../../components/chat/CommandMenu';
import {
  COMMAND_DEFINITIONS,
  buildUserCommandDefinitions,
  type CommandDefinition,
} from '../../utils/chatCommands';
import { useAppStore } from '../../stores/AppStore';

interface Props {
  open: boolean;
  onClose: () => void;
  isModerator: boolean;
  isBroadcaster: boolean;
  /** Insert `/name ` into the composer. */
  onPick: (cmd: CommandDefinition) => void;
}

export const MobileCommandSheet: React.FC<Props> = ({ open, onClose, isModerator, isBroadcaster, onPick }) => {
  const userCommands = useAppStore((s) => s.settings.chat_commands?.user_commands);
  const commands = useMemo(
    () => [...COMMAND_DEFINITIONS, ...buildUserCommandDefinitions(userCommands)],
    [userCommands],
  );
  const [q, setQ] = useState('');
  const visible = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return commands.filter((c) => matches(c, needle));
  }, [commands, q]);

  return (
    <MobileSheet open={open} onClose={onClose} title="Chat commands">
      <div className="px-4 pb-2">
        <input
          className="glass-input w-full h-10 px-3 text-[14px]"
          placeholder="Search commands"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
        />
      </div>
      <div className="overflow-y-auto min-h-0 flex-1 px-2 pb-3">
        {visible.length === 0 && (
          <div className="px-2 py-8 text-center text-[13px] text-textMuted">
            Nothing matches. Try a word from what you want to do, like ban, slow or reminder.
          </div>
        )}
        {visible.map((cmd, i) => {
          const first = i === 0 || visible[i - 1].category !== cmd.category;
          const lock = lockFor(cmd, isModerator, isBroadcaster);
          return (
            <React.Fragment key={`${cmd.category}:${cmd.name}`}>
              {first && (
                <div className="px-2 pt-3 pb-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-textMuted">
                  {RAIL.find((r) => r.key === cmd.category)?.label ?? cmd.category}
                </div>
              )}
              <button
                onClick={() => {
                  if (lock) return;
                  onPick(cmd);
                  onClose();
                }}
                disabled={!!lock}
                className={`sn-touch w-full text-left px-2 py-2.5 rounded-xl flex items-start gap-3 ${
                  lock ? 'opacity-50' : 'active:bg-white/5'
                }`}
              >
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] font-medium text-textPrimary">/{cmd.name}</div>
                  <div className="text-[12.5px] text-textSecondary leading-snug">{cmd.description}</div>
                  {lock && <div className="text-[11.5px] text-textMuted mt-0.5">{lock.line}</div>}
                </div>
                {lock && <Lock size={14} className="shrink-0 mt-1 text-textMuted" />}
              </button>
            </React.Fragment>
          );
        })}
      </div>
    </MobileSheet>
  );
};
