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
  /** The emote panel's field: 32px, the iPhone's, so the panel in the
   *  keyboard's place keeps its height for the grid. */
  compact?: boolean;
}

export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(
  ({ value, onChange, placeholder, className = '', compact = false, onKeyDown, ...input }, ref) => (
    <div
      className={`glass-input relative flex items-center ${compact ? 'h-8 !rounded-[10px]' : 'h-11 !rounded-full'} ${className}`}
    >
      <MagnifyingGlass
        size={compact ? 14 : 18}
        aria-hidden
        className={`pointer-events-none absolute text-textSecondary ${compact ? 'left-2.5' : 'left-3.5'}`}
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
        className={`h-full w-full min-w-0 bg-transparent text-textPrimary outline-none placeholder:text-textSecondary [&::-webkit-search-cancel-button]:hidden ${
          compact ? 'pl-8 pr-8 text-[13px]' : 'pl-10 pr-10 text-[15px]'
        }`}
        {...input}
      />
      {value && (
        <button
          type="button"
          // Clearing keeps the keyboard for the next search: the press never
          // takes focus from the field.
          onPointerDown={(e) => e.preventDefault()}
          data-keeps-keyboard=""
          onClick={() => onChange('')}
          aria-label="Clear search"
          className={`absolute flex items-center justify-center text-textSecondary active:opacity-70 ${
            compact ? 'right-0 h-8 w-8' : 'right-1 h-9 w-9'
          }`}
        >
          <XCircle size={compact ? 15 : 18} weight="fill" />
        </button>
      )}
    </div>
  ),
);
SearchField.displayName = 'SearchField';
