import React from 'react';
import { Plus, Users } from 'lucide-react';
import { ChannelItem, DEFAULT_AVATAR } from './channelSearch';
import { ProviderLogo } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';

/** One row in a channel smart-list. Renders live follows and search hits from
 *  every platform identically, with an optional trailing slot for a non-add
 *  affordance (e.g. a check when the channel is already in the preset).
 *
 *  A row can be disabled for two different reasons and they read differently to
 *  the user: `disabled` alone is transient (an add is already in flight), while
 *  `reason` means this channel can never be added here and says why on hover. */
export const ChannelResultRow: React.FC<{
  item: ChannelItem;
  index: number;
  highlighted: boolean;
  disabled?: boolean;
  /** Why this row cannot be picked. Shown on hover; also disables the row. */
  reason?: string | null;
  /** Override the trailing affordance (defaults to a + add indicator). */
  trailing?: React.ReactNode;
  onSelect: (item: ChannelItem) => void;
  onHover: (index: number) => void;
}> = ({ item, index, highlighted, disabled = false, reason, trailing, onSelect, onHover }) => {
  const blocked = !!reason;
  const row = (
    <button
      data-idx={index}
      onClick={() => onSelect(item)}
      onMouseEnter={() => onHover(index)}
      disabled={disabled || blocked}
      className={`w-full px-2.5 py-2 text-left rounded-lg transition-all duration-150 flex items-center gap-3 group disabled:opacity-40 ${
        highlighted ? 'bg-white/[0.06]' : 'hover:bg-white/[0.06]'
      }`}
    >
      {/* Avatar with accent ring when active */}
      <div className="relative shrink-0">
        {item.avatarUrl ? (
          <img
            src={item.avatarUrl}
            alt={item.displayName}
            onError={(e) => {
              const img = e.currentTarget as HTMLImageElement;
              if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
            }}
            className={`w-8 h-8 rounded-full object-cover ring-2 transition-all duration-200 shadow-sm ${
              highlighted ? 'ring-accent/30' : 'ring-transparent group-hover:ring-accent/30'
            }`}
          />
        ) : (
          <div
            className={`w-8 h-8 rounded-full bg-white/[0.04] ring-2 flex items-center justify-center transition-all duration-200 ${
              highlighted ? 'ring-accent/30' : 'ring-transparent group-hover:ring-accent/30'
            }`}
          >
            <Users size={13} className="text-textSecondary" />
          </div>
        )}
        {/* Live dot on avatar */}
        {item.isLive && (
          <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-red-500 border-2 border-surface/80"></span>
        )}
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <span
          className={`flex items-center gap-1.5 text-[13px] font-semibold leading-tight transition-colors ${
            highlighted ? 'text-accent' : 'text-textPrimary group-hover:text-accent'
          }`}
        >
          {/* Twitch is the unmarked default, same as everywhere else in the app:
              a mark on every row would be noise rather than information. */}
          {item.provider && item.provider !== 'twitch' && (
            <ProviderLogo provider={item.provider} size={11} className="shrink-0" />
          )}
          <span className="truncate">{item.displayName}</span>
        </span>
        <span className="block text-[11px] text-textMuted truncate mt-0.5 leading-tight">
          {item.isLive && item.gameName ? item.gameName : item.isLive ? 'Live' : item.login}
        </span>
      </div>

      {/* Trailing affordance, defaults to an add indicator. A blocked row shows
          nothing here: an add button that cannot add is worse than no button. */}
      {trailing !== undefined ? (
        trailing
      ) : blocked ? null : (
        <div
          className={`w-6 h-6 rounded-full flex items-center justify-center transition-all duration-200 shrink-0 ${
            highlighted ? 'bg-accent/15' : 'bg-transparent group-hover:bg-accent/15'
          }`}
        >
          <Plus
            size={13}
            className={`transition-colors ${highlighted ? 'text-accent' : 'text-textMuted group-hover:text-accent'}`}
          />
        </div>
      )}
    </button>
  );

  // A disabled button emits no pointer events, so the tooltip has to wrap it
  // rather than live on it, or the reason can never be read.
  return blocked ? (
    <Tooltip content={reason} side="right">
      <div className="w-full">{row}</div>
    </Tooltip>
  ) : (
    row
  );
};
