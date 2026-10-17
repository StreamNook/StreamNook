// The segmented row at the bottom of the chat box's menus (the insert menu,
// the wallet): one button per section, icon and label, right above the box.
import type { ReactNode } from 'react';
import { Tooltip } from '../ui/Tooltip';

export interface MenuSwitcherTab<K extends string> {
  key: K;
  label: string;
  icon?: ReactNode;
  /** Said on hover, for a section with more to say than its label. */
  hint?: string;
}

interface Props<K extends string> {
  /** What the row switches between, for screen readers ("Insert", "Wallet"). */
  label: string;
  tabs: MenuSwitcherTab<K>[];
  value: K;
  onChange: (key: K) => void;
}

export function MenuSwitcher<K extends string>({ label, tabs, value, onChange }: Props<K>) {
  return (
    <div role="tablist" aria-label={label} className="flex flex-shrink-0 gap-1 border-t border-borderSubtle p-1.5">
      {tabs.map((tab) => {
        const active = value === tab.key;
        const button = (
          <button
            key={tab.key}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(tab.key)}
            className={`flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-lg py-1.5 text-[12px] font-semibold transition-colors ${
              active ? 'glass-button-active text-accent' : 'text-textSecondary hover:bg-white/5 hover:text-textPrimary'
            }`}
          >
            {tab.icon}
            <span className="truncate">{tab.label}</span>
          </button>
        );
        return tab.hint ? (
          <Tooltip key={tab.key} content={tab.hint} side="top">
            {button}
          </Tooltip>
        ) : (
          button
        );
      })}
    </div>
  );
}
