// The chat command browser, phone shape. Rust builds the list
// (`command_catalog`): the built-in table without the desktop-only commands,
// then your own commands from Settings, each row locked for your role in this
// room and filtered by the search. This sheet only draws it.
import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Lock } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

/** One row of `command_catalog` (services/command_catalog.rs). */
interface CatalogRow {
  name: string;
  usage: string;
  description: string;
  category: string;
  /** The section header the row sits under. */
  label: string;
  /** What the row needs when your role here does not allow it. */
  lock?: string;
}

interface Props {
  open: boolean;
  onClose: () => void;
  /** The room's bare Twitch login. */
  channel: string;
  /** Insert `/name ` into the composer. */
  onPick: (cmd: { name: string }) => void;
}

/** One section per header label, in the order the catalog first names each,
 *  so a category the catalog interleaves never prints its header twice. */
function sectionsOf(rows: CatalogRow[]): { label: string; rows: CatalogRow[] }[] {
  const byLabel = new Map<string, CatalogRow[]>();
  for (const row of rows) {
    const group = byLabel.get(row.label);
    if (group) group.push(row);
    else byLabel.set(row.label, [row]);
  }
  return Array.from(byLabel, ([label, group]) => ({ label, rows: group }));
}

export const MobileCommandSheet: React.FC<Props> = ({ open, onClose, channel, onPick }) => {
  // Rust reads your own commands from settings; a change re-asks.
  const userCommands = useAppStore((s) => s.settings.chat_commands?.user_commands);
  const [q, setQ] = useState('');
  const [visible, setVisible] = useState<CatalogRow[]>([]);
  const sections = useMemo(() => sectionsOf(visible), [visible]);

  useEffect(() => {
    if (!open) return;
    let live = true;
    invoke<CatalogRow[]>('command_catalog', { channel, platform: 'phone', query: q.trim() })
      .then((rows) => {
        if (live) setVisible(rows);
      })
      .catch((err) => Logger.warn('[MobileCommandSheet] command_catalog failed:', err));
    return () => {
      live = false;
    };
  }, [open, channel, q, userCommands]);

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
        {sections.map((section) => (
          <section key={section.label}>
            <div className="px-2 pt-3 pb-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-textMuted">
              {section.label}
            </div>
            {section.rows.map((cmd) => {
              const lock = cmd.lock;
              return (
                <button
                  key={`${cmd.category}:${cmd.name}`}
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
                    {lock && <div className="text-[11.5px] text-textMuted mt-0.5">{lock}</div>}
                  </div>
                  {lock && <Lock size={14} className="shrink-0 mt-1 text-textMuted" />}
                </button>
              );
            })}
          </section>
        ))}
      </div>
    </MobileSheet>
  );
};
