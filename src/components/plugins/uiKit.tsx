// The reusable settings controls the host shares with UI plugins through
// `api.components`. A plugin's own React panel renders these so its settings
// look and behave exactly like the app's native ones (rich channel pickers,
// chip lists, sliders) instead of a poorer rebuild. The generic
// PluginPanelRenderer draws from the same set, so the two never drift.

import { useState, type ReactNode } from 'react';
import { Plus, X } from 'lucide-react';
import { Logger } from '../../utils/logger';
import PanelChannelList from './PanelChannelList';
import { Toggle } from '../ui/Toggle';
import { SettingsSection as PageSection, SettingsRow as PageRow, SegmentedSelect } from '../settings/_primitives';
import {
  SettingsPage,
  PillList,
  InlineSlider,
  SubControls,
  SubControl,
  DropsSettingsTab,
} from './settingsPageKit';
import type { PanelChannel } from '../../types/plugins';

export { Toggle };

// Add-and-remove chip rows for a list of strings (not a textarea).
export const ChipList = ({
  items,
  placeholder,
  onChange,
}: {
  items: string[];
  placeholder?: string;
  onChange: (next: string[]) => void;
}) => {
  const [input, setInput] = useState('');
  const add = () => {
    const v = input.trim();
    if (v && !items.includes(v)) onChange([...items, v]);
    setInput('');
  };
  return (
    <div className="mt-2">
      <div className="mb-2 space-y-1.5">
        {items.length > 0 ? (
          items.map((item, i) => (
            <div
              key={item}
              className="group flex items-center gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2"
            >
              <span className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded bg-white/5 font-mono text-[11px] text-textSecondary">
                {i + 1}
              </span>
              <span className="min-w-0 flex-1 truncate text-[13px] text-textPrimary">{item}</span>
              <button
                type="button"
                onClick={() => onChange(items.filter((x) => x !== item))}
                className="rounded p-1 text-textMuted opacity-0 transition-all hover:bg-red-500/10 hover:text-red-300 group-hover:opacity-100"
              >
                <X size={14} />
              </button>
            </div>
          ))
        ) : (
          <div className="rounded-lg border border-dashed border-white/10 bg-white/[0.02] px-3 py-2.5 text-center text-[12px] italic text-textSecondary">
            None added.
          </div>
        )}
      </div>
      <div className="flex gap-2">
        <input
          type="text"
          value={input}
          placeholder={placeholder ?? 'Add...'}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && add()}
          className="glass-input flex-1 rounded-md px-3 py-1.5 text-[13px] text-textPrimary"
        />
        <button
          type="button"
          onClick={add}
          disabled={!input.trim()}
          className="glass-button-secondary flex items-center gap-1 px-3 py-1.5 text-[13px] text-textSecondary hover:text-textPrimary disabled:opacity-50"
        >
          <Plus size={14} />
          Add
        </button>
      </div>
    </div>
  );
};

// A directory path with a native browse dialog. Empty string means a caller-
// defined default; the placeholder describes what that default is.
export const FolderPicker = ({
  value,
  placeholder,
  onChange,
}: {
  value: string;
  placeholder?: string;
  onChange: (next: string) => void;
}) => {
  const browse = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === 'string' && picked) onChange(picked);
    } catch (err) {
      Logger.error('[uiKit] folder pick failed:', err);
    }
  };
  return (
    <div className="mt-2 flex items-center gap-2">
      <div className="glass-input min-w-0 flex-1 truncate rounded-md px-3 py-1.5 text-[13px]">
        {value ? (
          <span className="text-textPrimary">{value}</span>
        ) : (
          <span className="italic text-textSecondary">{placeholder ?? 'Default'}</span>
        )}
      </div>
      <button
        type="button"
        onClick={browse}
        className="glass-button-secondary flex-shrink-0 px-3 py-1.5 text-[13px] text-textSecondary hover:text-textPrimary"
      >
        Browse
      </button>
      {value && (
        <button
          type="button"
          onClick={() => onChange('')}
          className="flex-shrink-0 rounded-md px-2 py-1.5 text-[13px] text-textMuted transition-colors hover:bg-white/5 hover:text-textPrimary"
        >
          Reset
        </button>
      )}
    </div>
  );
};

export const Slider = ({
  value,
  min,
  max,
  step,
  unit,
  displayDivisor,
  onChange,
}: {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  /** Divide the stored value by this for display (e.g. seconds shown as minutes). */
  displayDivisor?: number;
  onChange: (n: number) => void;
}) => {
  const divisor = displayDivisor ?? 1;
  const shown = divisor > 1 ? Math.round(value / divisor) : value;
  return (
    <div className="mt-1">
      <div className="mb-1 text-[12px] text-textSecondary">
        {shown}
        {unit ? ` ${unit}` : ''}
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step ?? 1}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full cursor-pointer accent-accent"
      />
    </div>
  );
};

// Native option dropdown matching the app's glass styling.
export const Select = ({
  value,
  options,
  onChange,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (next: string) => void;
}) => (
  <select
    className="glass-input rounded-md bg-transparent px-2 py-1 text-[13px] text-textPrimary"
    value={value}
    onChange={(e) => onChange(e.target.value)}
  >
    {options.map((opt) => (
      <option key={opt.value} value={opt.value} className="bg-zinc-900">
        {opt.label}
      </option>
    ))}
  </select>
);

// The app's channel search-picker (avatars, live dots, removable rows).
export const ChannelList = ({
  value,
  onChange,
}: {
  value: PanelChannel[];
  onChange: (next: PanelChannel[]) => void;
}) => <PanelChannelList value={value} onChange={onChange} />;

// Layout primitives matching the native settings look: a titled section, an
// inline label+control row, and a full-width block for the larger controls.
export const SettingsSection = ({
  label,
  description,
  children,
}: {
  label?: string;
  description?: string;
  children: ReactNode;
}) => (
  <div className="rounded-lg bg-white/[0.02] py-1.5">
    {label && (
      <div className="px-3 pb-0.5 pt-1.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-textMuted">
        {label}
      </div>
    )}
    {description && (
      <p className="px-3 pb-1 text-[12px] leading-relaxed text-textSecondary">{description}</p>
    )}
    {children}
  </div>
);

export const SettingsRow = ({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: ReactNode;
}) => (
  <div className="settings-row px-3 py-2.5">
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-textPrimary">{label}</div>
        {description && (
          <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">{description}</p>
        )}
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  </div>
);

export const SettingsBlock = ({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: ReactNode;
}) => (
  <div className="settings-row px-3 py-2.5">
    <div className="text-[13px] font-medium text-textPrimary">{label}</div>
    {description && (
      <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">{description}</p>
    )}
    {children}
  </div>
);

// The bundle of controls shared with plugins via `api.components`. The
// SettingsSection/Row/Block trio is the compact panel layout; SettingsPage and
// the Page* parts are the Settings window's own look, for a plugin that ships
// a full page (see settingsPageKit.tsx). DropsSettingsTab only answers
// Autopilot 0.4.x, which drew the app's page by that name.
export const UI_KIT = {
  Toggle,
  ChipList,
  FolderPicker,
  Slider,
  Select,
  ChannelList,
  SettingsSection,
  SettingsRow,
  SettingsBlock,
  SettingsPage,
  PageSection,
  PageRow,
  SegmentedSelect,
  PillList,
  InlineSlider,
  SubControls,
  SubControl,
  DropsSettingsTab,
};

export type UiKit = typeof UI_KIT;
