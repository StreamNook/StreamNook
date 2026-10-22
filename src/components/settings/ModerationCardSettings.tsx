// Settings > Moderation on the phone: swiping a message to moderate it (on or
// off, its look, its reach and its steps), the four timeout lengths on a user
// card (the desktop hover dock's first four, so one edit changes both) and the
// viewer's own card buttons. Desktop's Moderation page mounts the user card
// part on its own. Rust keeps every list (`edit_mod_slider_timeouts`,
// `set_card_timeout_length`, `edit_settings_list`, one change at a time) and
// reads typed lengths (`parse_mod_timeout`).
import { useMemo, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import type { ModCardAction } from '../../types';
import { SegmentedSelect, SettingsRow, SettingsSection, SettingsSubGroup } from './_primitives';
import { Toggle } from '../ui/Toggle';
import { parseModTimeout } from '../../services/moderation';
import { EntryInput } from './entryInput';
import { useSettingReset } from './settingReset';
import { IS_MOBILE } from '../../utils/platform';

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

/** A length as every step names it: one unit when it divides evenly ("10m",
 *  "1d"), else the two largest parts ("1h 30m"), as Rust's duration_label. */
function lengthLabel(seconds: number): string {
  const units: [number, string][] = [[604800, 'w'], [86400, 'd'], [3600, 'h'], [60, 'm'], [1, 's']];
  const first = units.find(([size]) => seconds >= size);
  if (first && seconds % first[0] === 0) return `${seconds / first[0]}${first[1]}`;
  const parts: string[] = [];
  let rest = seconds;
  for (const [size, unit] of units) {
    if (rest >= size && parts.length < 2) {
      parts.push(`${Math.floor(rest / size)}${unit}`);
      rest %= size;
    }
  }
  return parts.length ? parts.join(' ') : '0s';
}

const CUSTOM = -1;

/** A small field for a typed length ("45m", "1h30m", "2d"), read by Rust.
 *  Enter adds it; an unreadable entry keeps the field with a red edge, Rust's
 *  words under it and the text selected for the next try; Escape gives up. */
function TypedLength({ onLength, onDone }: { onLength: (seconds: number) => void; onDone: () => void }) {
  const [text, setText] = useState('');
  const [refusal, setRefusal] = useState<string | null>(null);
  const ref = useRef<HTMLInputElement>(null);
  const submit = async () => {
    try {
      const step = await parseModTimeout(text);
      if (step.seconds) onLength(step.seconds);
      onDone();
    } catch (err) {
      setRefusal(typeof err === 'string' ? err : 'Type a length like 45m, 2h or 3d, up to 14 days.');
      ref.current?.select();
    }
  };
  return (
    <div>
      <input
        ref={ref}
        autoFocus
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setRefusal(null);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void submit();
          if (e.key === 'Escape') onDone();
        }}
        onBlur={() => !text.trim() && onDone()}
        placeholder="45m"
        enterKeyHint="done"
        autoCapitalize="off"
        autoCorrect="off"
        className={`h-9 w-24 rounded-[10px] border bg-surface px-2.5 text-[14px] text-textPrimary outline-none placeholder:text-textMuted ${
          refusal ? 'border-error' : 'border-transparent'
        }`}
      />
      {refusal && <p className="mt-1 text-[12px] text-error">{refusal}</p>}
    </div>
  );
}

/** A length picker: the usual lengths, the one set now (even a typed one),
 *  and "Custom length…" first. */
function LengthSelect({
  value,
  onPick,
  taken = [],
  className,
  label,
}: {
  value: number;
  onPick: (seconds: number) => void;
  taken?: number[];
  className: string;
  label: string;
}) {
  const [typing, setTyping] = useState(false);
  if (typing) return <TypedLength onLength={onPick} onDone={() => setTyping(false)} />;
  const shown = LENGTHS.some(([v]) => v === value) ? LENGTHS : [...LENGTHS, [value, lengthLabel(value)] as [number, string]].sort((a, b) => a[0] - b[0]);
  return (
    <select
      aria-label={label}
      value={value}
      onChange={(e) => {
        const next = Number(e.target.value);
        if (next === CUSTOM) setTyping(true);
        else onPick(next);
      }}
      className={className}
    >
      <option value={CUSTOM}>Custom length…</option>
      {shown.map(([v, l]) => (
        <option key={v} value={v} disabled={v !== value && taken.includes(v)}>
          {l}
        </option>
      ))}
    </select>
  );
}

const setFields = (changes: Record<string, unknown>) =>
  invoke('set_settings_fields', { changes }).catch((err) => Logger.warn('[Moderation] setting not saved:', err));

/** Moderating Messages: the swipe (its style, reach and steps) and the hold
 *  menu's Delete, Timeout and Ban, each on or off. */
function SwipeModerateSettings() {
  const design = useAppStore((s) => s.settings.chat_design);
  const resetFor = useSettingReset();
  const on = design?.mod_slider ?? true;
  const hold = design?.mod_hold_buttons ?? true;
  const timeouts = design?.mod_slider_timeouts ?? [60, 300, 600, 1800, 3600, 86400, 604800];
  const withDelete = design?.mod_slider_delete ?? true;
  const withBan = design?.mod_slider_ban ?? true;
  const confirmBans = useAppStore((s) => s.settings.moderation?.confirm_bans ?? false);
  const [adding, setAdding] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);
  const edit = async (op: 'add' | 'remove', seconds: number) => {
    try {
      await invoke('edit_mod_slider_timeouts', { op, seconds });
      setRefusal(null);
    } catch (err) {
      setRefusal(typeof err === 'string' ? err : 'That length could not be saved');
    }
  };
  const chip = 'flex h-8 items-center gap-1 rounded-full px-3 text-[13px] font-semibold';
  return (
    <SettingsSection
      label="Moderating Messages"
      description="Two ways to delete, time out or ban from a message in a channel you moderate. With both off, the user card’s Moderate row still has every action."
    >
      <SettingsRow
        title="Swipe to moderate"
        description="Swipe right on a message; pull further for a longer timeout. Ban needs a short hold."
        control={<Toggle enabled={on} onChange={() => void setFields({ 'chat_design.mod_slider': !on })} />}
        onReset={resetFor(['chat_design.mod_slider', true])}
      />
      {on && (
        <SettingsSubGroup>
          <SettingsRow
            title="Style"
            onReset={resetFor(['chat_design.mod_slider_style', 'preview'])}
            control={
              <SegmentedSelect
                value={(design?.mod_slider_style as 'preview' | 'rail' | 'capsule') ?? 'preview'}
                options={[
                  { value: 'preview', label: 'Preview' },
                  { value: 'rail', label: 'Rail' },
                  { value: 'capsule', label: 'Capsule' },
                ]}
                onChange={(v) => void setFields({ 'chat_design.mod_slider_style': v })}
              />
            }
          />
          <SettingsRow
            title="Length"
            onReset={resetFor(['chat_design.mod_slider_length', 'medium'])}
            control={
              <SegmentedSelect
                value={(design?.mod_slider_length as 'short' | 'medium' | 'long') ?? 'medium'}
                options={[
                  { value: 'short', label: 'Short' },
                  { value: 'medium', label: 'Medium' },
                  { value: 'long', label: 'Long' },
                ]}
                onChange={(v) => void setFields({ 'chat_design.mod_slider_length': v })}
              />
            }
          />
          <SettingsRow title="Steps" description="What the swipe offers, mildest first. Tap a length to take it out.">
            <div className="flex flex-wrap items-center gap-[7px]">
              <button
                type="button"
                onClick={() => void setFields({ 'chat_design.mod_slider_delete': !withDelete })}
                className={`${chip} ${withDelete ? 'bg-surface text-textPrimary' : 'border border-textMuted/40 text-textMuted'}`}
                aria-pressed={withDelete}
              >
                Delete
              </button>
              {timeouts.map((secs) => (
                <button
                  key={secs}
                  type="button"
                  disabled={timeouts.length <= 1}
                  onClick={() => void edit('remove', secs)}
                  className={`${chip} bg-surface text-textPrimary`}
                  aria-label={`Remove ${lengthLabel(secs)}`}
                >
                  {lengthLabel(secs)}
                  {timeouts.length > 1 && <span className="text-[10px] text-textMuted">✕</span>}
                </button>
              ))}
              {timeouts.length < 8 &&
                (adding ? (
                  <TypedLength onLength={(secs) => void edit('add', secs)} onDone={() => setAdding(false)} />
                ) : (
                  <select
                    aria-label="Add a length"
                    value=""
                    onChange={(e) => {
                      const next = Number(e.target.value);
                      if (next === CUSTOM) setAdding(true);
                      else if (next) void edit('add', next);
                    }}
                    className="h-8 w-[72px] appearance-none rounded-full border border-dashed border-accent/40 bg-transparent px-3 text-center text-[13px] font-medium text-accent outline-none"
                  >
                    <option value="" disabled>
                      + Add
                    </option>
                    <option value={CUSTOM}>Custom length…</option>
                    {LENGTHS.filter(([v]) => !timeouts.includes(v)).map(([v, l]) => (
                      <option key={v} value={v}>
                        {l}
                      </option>
                    ))}
                  </select>
                ))}
              <button
                type="button"
                onClick={() => void setFields({ 'chat_design.mod_slider_ban': !withBan })}
                className={`${chip} ${withBan ? 'bg-error/[0.14] text-error' : 'border border-textMuted/40 text-textMuted'}`}
                aria-pressed={withBan}
              >
                Ban
              </button>
            </div>
            {refusal && <p className="mt-1.5 text-[12px] text-error">{refusal}</p>}
          </SettingsRow>
        </SettingsSubGroup>
      )}
      <SettingsRow
        title="Hold menu buttons"
        description="Delete, Timeout and Ban in the press-and-hold menu."
        control={<Toggle enabled={hold} onChange={() => void setFields({ 'chat_design.mod_hold_buttons': !hold })} />}
        onReset={resetFor(['chat_design.mod_hold_buttons', true])}
      />
      <SettingsRow
        title="Ban confirmation"
        description="Ban on a user card waits for a second tap, so a slip never bans anyone. Swipe already asks for a hold."
        control={<Toggle enabled={confirmBans} onChange={() => void setFields({ 'moderation.confirm_bans': !confirmBans })} />}
        onReset={resetFor(['moderation.confirm_bans', false])}
      />
    </SettingsSection>
  );
}

type StoredAction = ModCardAction & { id: string };

async function editActions(edit: Record<string, unknown>): Promise<StoredAction[] | null> {
  try {
    return await invoke<StoredAction[]>('edit_settings_list', { path: PATH, edit });
  } catch (err) {
    Logger.warn('[ModerationCard] edit failed:', err);
    return null;
  }
}

// The phone draws filled fields; desktop draws its glass inputs, as the rows
// around this section do.
const selectClass = IS_MOBILE
  ? 'h-9 rounded-lg bg-surface px-2 text-[13px] font-medium text-textPrimary outline-none'
  : 'glass-input h-9 px-2.5 text-[13px] font-medium text-textPrimary';
const fieldClass = IS_MOBILE
  ? 'h-9 rounded-lg bg-surface px-2.5 text-[13px] text-textPrimary outline-none placeholder:text-textMuted'
  : 'glass-input h-9 px-3 text-[13px] text-textPrimary placeholder-textSecondary';

/** A field with its name above it. */
const FieldLabel = ({ text, children }: { text: string; children: ReactNode }) => (
  <label className="block min-w-0">
    <span className="mb-1.5 block text-[12px] font-medium text-textSecondary">{text}</span>
    {children}
  </label>
);
const lengthClass = IS_MOBILE
  ? 'h-9 min-w-0 flex-1 appearance-none rounded-[10px] bg-surface text-center text-[14px] font-semibold text-textPrimary outline-none'
  : 'glass-input h-8 min-w-0 flex-1 appearance-none text-center text-xs font-semibold text-textPrimary';

/** Settings > Moderation on the phone: the swipe, then the user card's
 *  buttons. Desktop mounts only the card section (`UserCardSettings`). */
const ModerationCardSettings = () => (
  <div className="space-y-6">
    <SwipeModerateSettings />
    <UserCardSettings />
  </div>
);

/** User Cards: the card's four timeout lengths and the viewer's own buttons. */
export const UserCardSettings = () => {
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
    <SettingsSection
      id="settings-section-user-cards"
      label="User Cards"
      description="The buttons on a user card in channels you moderate. Ban, and Warn when your sign-in allows it, always follow the lengths."
    >
      <SettingsRow
        title="Timeout lengths"
        description={`${IS_MOBILE ? 'Tap' : 'Pick'} one to change it. Every quick timeout uses the same four.`}
      >
        <div className="flex gap-[7px]">
          {four.map((seconds, index) => (
            <LengthSelect
              key={index}
              label={`Timeout length ${index + 1}`}
              value={seconds}
              taken={four}
              onPick={(next) => void setLength(index, next)}
              className={lengthClass}
            />
          ))}
        </div>
        {refusal && <p className="mt-1.5 text-[12px] text-error">{refusal}</p>}
      </SettingsRow>
      <SettingsRow
        title="Card buttons"
        description="Your own buttons under the lengths: a timeout, ban, unban or warning, with a reason sent along."
      >
        <div className="space-y-3">
          {actions.map((a) => (
            <div key={a.id} className="glass-panel space-y-3 p-3.5">
              {/* Every field named above it: three bare boxes read as a puzzle. */}
              <div className="flex items-start gap-3">
                <div
                  className={`min-w-0 flex-1 grid gap-3 ${
                    IS_MOBILE ? 'grid-cols-2 [&>*:first-child]:col-span-2' : 'grid-cols-[minmax(0,1fr)_auto_auto]'
                  }`}
                >
                  <FieldLabel text="Button label">
                    <EntryInput
                      value={a.label ?? ''}
                      save={saveField(a.id, 'label')}
                      maxLength={24}
                      placeholder="For example, Spoilers"
                      className={`${fieldClass} w-full`}
                    />
                  </FieldLabel>
                  <FieldLabel text="What it does">
                    <select
                      value={a.kind}
                      onChange={(e) => void editActions({ op: 'update', id: a.id, patch: { kind: e.target.value } })}
                      className={`${selectClass} w-full`}
                    >
                      {KINDS.map(([value, label]) => (
                        <option key={value} value={value}>
                          {label}
                        </option>
                      ))}
                    </select>
                  </FieldLabel>
                  {a.kind === 'timeout' && (
                    <FieldLabel text="Length">
                      <LengthSelect
                        label="Timeout length"
                        value={a.seconds ?? 600}
                        onPick={(next) => void editActions({ op: 'update', id: a.id, patch: { seconds: next } })}
                        className={`${selectClass} w-full min-w-[96px]`}
                      />
                    </FieldLabel>
                  )}
                </div>
                <button
                  type="button"
                  aria-label="Remove this button"
                  onClick={() => void editActions({ op: 'remove', id: a.id })}
                  className="mt-[22px] rounded-md p-2 text-textMuted transition-colors hover:bg-white/5 hover:text-error active:text-textPrimary"
                >
                  <Trash2 size={15} />
                </button>
              </div>
              <FieldLabel text={a.kind === 'warn' ? 'Reason they see (asked each time when empty)' : 'Reason they see (optional)'}>
                <EntryInput
                  value={a.reason ?? ''}
                  save={saveField(a.id, 'reason')}
                  maxLength={500}
                  placeholder="For example, no spoilers please"
                  className={`${fieldClass} w-full`}
                />
              </FieldLabel>
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
