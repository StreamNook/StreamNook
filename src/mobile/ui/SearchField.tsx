// The phone's one search field: a 44px capsule on the theme's raised surface
// (the field style in mobile.css), a magnifier on the leading edge, and a
// clear button once there is text. Browse, Rewards, settings search and the
// emote search each drew their own (three heights, two corner sizes, and the
// emote search had neither magnifier nor clear).
import React, { forwardRef } from 'react';
import { MagnifyingGlass, XCircle } from 'phosphor-react';

export interface SearchFieldProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'type' | 'className'> {
  value: string;
  onChange: (next: string) => void;
  placeholder: string;
  className?: string;
}

export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(
  ({ value, onChange, placeholder, className = '', onKeyDown, ...input }, ref) => (
    <div className={`glass-input relative flex h-11 items-center !rounded-full ${className}`}>
      <MagnifyingGlass
        size={18}
        aria-hidden
        className="pointer-events-none absolute left-3.5 text-textSecondary"
      />
      <input
        ref={ref}
        type="search"
        enterKeyHint="search"
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        // The Search key puts the keyboard away: results are already live as
        // you type, and scrolling them never closes it.
        onKeyDown={(e) => {
          onKeyDown?.(e);
          if (!e.defaultPrevented && e.key === 'Enter' && !e.nativeEvent.isComposing) e.currentTarget.blur();
        }}
        placeholder={placeholder}
        className="h-full w-full min-w-0 bg-transparent pl-10 pr-10 text-[15px] text-textPrimary outline-none placeholder:text-textSecondary [&::-webkit-search-cancel-button]:hidden"
        {...input}
      />
      {value && (
        <button
          type="button"
          onClick={() => onChange('')}
          aria-label="Clear search"
          className="absolute right-1 flex h-9 w-9 items-center justify-center text-textSecondary active:opacity-70"
        >
          <XCircle size={18} weight="fill" />
        </button>
      )}
    </div>
  ),
);
SearchField.displayName = 'SearchField';
