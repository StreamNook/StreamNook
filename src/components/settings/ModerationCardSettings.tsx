// Chat > Moderation on the phone: the four timeout lengths on a user card
// (the desktop hover dock's first four, so one edit changes both) and the
// viewer's own card buttons. Rust keeps both lists: a length through
// `set_card_timeout_length`, a button through `edit_settings_list`, one change
// at a time; services/chatter_card.rs turns them into the card's buttons.
import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import type { ModCardAction } from '../../types';
import { SettingsRow, SettingsSection } from './_primitives';
import { EntryInput } from './entryInput';

const PATH = 'moderation.card_actions';
const DEFAULT_LENGTHS = [1, 600, 3600, 86400];
const MAX_BUTTONS = 6;

/** Lengths offered for a chip or a timeout button, shortest first. */
const LENGTHS: [number, string][] = [
  [1, '1s'], [30, '30s'], [60, '1m'], [300, '5m'], [600, '10m'], [1800, '30m'], [3600, '1h'], [21600, '6h'],
  [43200, '12h'], [86400, '1d'], [259200, '3d'], [604800, '1w'], [1209600, '2w'],
];
const KINDS: [ModCardAction['kind'], string][] = [
  ['timeout', 'Timeout'], ['ban', 'Ban'], ['unban', 'Unban'], ['warn', 'Warning'],
];

const lengthLabel = (s: number) => LENGTHS.find(([v]) => v === s)?.[1] ?? `${s}s`;

type StoredAction = ModCardAction & { id: string };

async function editActions(edit: Record<string, unknown>): Promise<StoredAction[] | null> {
  try {
    return await invoke<StoredAction[]>('edit_settings_list', { path: PATH, edit });
  } catch (err) {
    Logger.warn('[ModerationCard] edit failed:', err);
    return null;
  }
}

const selectClass =
  'h-8 rounded-lg bg-surface px-2 text-[13px] font-medium text-textPrimary outline-none';

const ModerationCardSettings = () => {
  const moderation = useAppStore((s) => s.settings.moderation);
  const [refusal, setRefusal] = useState<string | null>(null);

  // The four as the card shows them; Rust's lengths_from decides the same.
  const four = useMemo(() => {
    const out: number[] = [];
    for (const s of [...(moderation?.timeout_presets ?? []), ...DEFAULT_LENGTHS]) {
      if (s >= 1 && s <= 1209600 && !out.includes(s)) out.push(s);
      if (out.length === 4) break;
    }
    return out;
  }, [moderation?.timeout_presets]);
  const actions = (moderation?.card_actions ?? []) as StoredAction[];

  const setLength = async (index: number, seconds: number) => {
    try {
      await invoke('set_card_timeout_length', { index, seconds });
      setRefusal(null);
    } catch (err) {
      setRefusal(typeof err === 'string' ? err : 'That length could not be saved');
    }
  };

  const saveField = (id: string, field: 'label' | 'reason') => async (text: string) => {
    const stored = await editActions({ op: 'update', id, patch: { [field]: text } });
    return stored?.find((a) => a.id === id)?.[field];
  };

  return (
    <SettingsSection label="Moderation" description="The buttons on a user card in channels you moderate.">
      <SettingsRow title="Timeout lengths" description="Tap one to change it. The desktop's hover dock uses the same four.">
        <div className="flex gap-[7px]">
          {four.map((seconds, index) => (
            <select
              key={index}
              aria-label={`Timeout length ${index + 1}`}
              value={seconds}
              onChange={(e) => void setLength(index, Number(e.target.value))}
              className="h-9 min-w-0 flex-1 appearance-none rounded-[10px] bg-surface text-center text-[14px] font-semibold text-textPrimary outline-none"
            >
              {LENGTHS.map(([value, label]) => (
                <option key={value} value={value} disabled={value !== seconds && four.includes(value)}>
                  {label}
                </option>
              ))}
            </select>
          ))}
        </div>
        {refusal && <p className="mt-1.5 text-[12px] text-error">{refusal}</p>}
      </SettingsRow>
      <SettingsRow
        title="Card buttons"
        description="Your own buttons under the lengths: a timeout, ban, unban or warning, with a reason sent along."
      >
        <div className="space-y-2">
          {actions.map((a) => (
            <div key={a.id} className="glass-panel space-y-2 p-2.5">
              <div className="flex items-center gap-2">
                <EntryInput
                  value={a.label ?? ''}
                  save={saveField(a.id, 'label')}
                  maxLength={24}
                  placeholder="Label, like Spoilers"
                  className="h-8 min-w-0 flex-1 rounded-lg bg-surface px-2.5 text-[13px] text-textPrimary outline-none placeholder:text-textMuted"
                />
                <button
                  type="button"
                  aria-label="Remove this button"
                  onClick={() => void editActions({ op: 'remove', id: a.id })}
                  className="p-1.5 text-textMuted active:text-textPrimary"
                >
                  <Trash2 size={15} />
                </button>
              </div>
              <div className="flex items-center gap-2">
                <select
                  aria-label="What it does"
                  value={a.kind}
                  onChange={(e) => void editActions({ op: 'update', id: a.id, patch: { kind: e.target.value } })}
                  className={selectClass}
                >
                  {KINDS.map(([value, label]) => (
                    <option key={value} value={value}>
                      {label}
                    </option>
                  ))}
                </select>
                {a.kind === 'timeout' && (
                  <select
                    aria-label="Timeout length"
                    value={a.seconds ?? 600}
                    onChange={(e) => void editActions({ op: 'update', id: a.id, patch: { seconds: Number(e.target.value) } })}
                    className={selectClass}
                  >
                    {LENGTHS.map(([value]) => (
                      <option key={value} value={value}>
                        {lengthLabel(value)}
                      </option>
                    ))}
                  </select>
                )}
              </div>
              <EntryInput
                value={a.reason ?? ''}
                save={saveField(a.id, 'reason')}
                maxLength={500}
                placeholder={a.kind === 'warn' ? 'Reason (asked each time when empty)' : 'Reason (optional)'}
                className="h-8 w-full rounded-lg bg-surface px-2.5 text-[13px] text-textPrimary outline-none placeholder:text-textMuted"
              />
            </div>
          ))}
          {actions.length < MAX_BUTTONS && (
            <button
              type="button"
              onClick={() => void editActions({ op: 'add' })}
              className="flex h-9 items-center gap-1.5 text-[13.5px] font-medium text-accent"
            >
              <Plus size={15} />
              Add a button
            </button>
          )}
        </div>
      </SettingsRow>
    </SettingsSection>
  );
};

export default ModerationCardSettings;
