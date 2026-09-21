import React, { useEffect, useRef, useMemo } from 'react';
import { motion } from 'framer-motion';
import { ChatUser } from '../stores/chatUserStore';
import { computePaintStyle } from '../services/seventvService';
import { useAppStore } from '../stores/AppStore';
import { getDisplayedName, getColorOverride } from '../utils/userChatOverrides';
import { useNameColorAdjust } from '../hooks/useNameColor';
import type { UserChatOverride } from '../types';

interface MentionAutocompleteProps {
  /** List of matching users to display */
  users: ChatUser[];
  /** Currently selected index */
  selectedIndex: number;
  /** Callback when a user is selected */
  onSelect: (user: ChatUser) => void;
  /** Callback to change selected index */
  onSelectedIndexChange: (index: number) => void;
}

/**
 * Individual user item with paint styling
 */
const MentionUserItem: React.FC<{
  user: ChatUser;
  isSelected: boolean;
  onSelect: () => void;
  onHover: () => void;
  itemRef: (el: HTMLButtonElement | null) => void;
  overrides: Record<string, UserChatOverride> | undefined;
}> = ({ user, isSelected, onSelect, onHover, itemRef, overrides }) => {
  // Override-aware base color: the user's set color wins over their Twitch
  // color. 7TV paint (if any) still renders on top.
  const adjustNameColor = useNameColorAdjust();
  const effectiveColor = adjustNameColor(getColorOverride(user.userId, overrides) ?? user.color);

  // Compute paint style for the user's display name
  const nameStyle = useMemo(() => {
    if (user.paint) {
      return computePaintStyle(user.paint, effectiveColor);
    }
    return { color: effectiveColor || '#9147FF' };
  }, [user.paint, effectiveColor]);

  // Effective display label resolves to the user's nickname when one is set.
  // The @insertion still uses user.username, which Twitch IRC needs.
  const displayed = getDisplayedName(user.userId, user.displayName, overrides);
  const showRealName = displayed.toLowerCase() !== user.username.toLowerCase();

  return (
    <button
      ref={itemRef}
      className={`w-full px-3 py-2 flex items-center gap-2 text-left transition-colors ${
        isSelected
          ? 'bg-accent/20'
          : 'hover:bg-white/5'
      }`}
      onClick={onSelect}
      onMouseEnter={onHover}
    >
      {/* Color indicator dot */}
      <span
        className="w-2 h-2 rounded-full flex-shrink-0"
        style={{ backgroundColor: effectiveColor || '#9147FF' }}
      />
      {/* Display name with paint styling */}
      <span className="flex-1 min-w-0 truncate">
        <span
          className="font-semibold"
          style={nameStyle}
        >
          {displayed}
        </span>
        {showRealName && (
          <span className="text-textSecondary text-xs ml-1 opacity-70">
            (@{user.username})
          </span>
        )}
      </span>
    </button>
  );
};

/**
 * Floating autocomplete popup for @ mentions.
 * Shows matching users with keyboard navigation support.
 */
const MentionAutocomplete: React.FC<MentionAutocompleteProps> = ({
  users,
  selectedIndex,
  onSelect,
  onSelectedIndexChange,
}) => {
  const listRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const overrides = useAppStore((s) => s.settings.chat_customization?.user_overrides);

  // Scroll selected item into view
  useEffect(() => {
    const selectedItem = itemRefs.current[selectedIndex];
    if (selectedItem && listRef.current) {
      selectedItem.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex]);

  // Reset refs when users change
  useEffect(() => {
    itemRefs.current = itemRefs.current.slice(0, users.length);
  }, [users.length]);

  if (users.length === 0) {
    return null;
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 6, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: "easeOut" }}
      className="sn-popover absolute z-[60] w-full max-h-[220px] overflow-y-auto custom-scrollbar origin-bottom"
      style={{
        bottom: '100%',
        left: 0,
        right: 0,
        marginBottom: '8px',
      }}
      ref={listRef as any}
    >
      {/* Header */}
      <div className="px-3 py-1.5 border-b border-white/5 sticky top-0 z-10 bg-background/[0.5] backdrop-blur-md">
        <span className="text-[10px] font-medium text-white/50 uppercase tracking-wide">
          Mention User
        </span>
      </div>
      {/* User list */}
      <div className="py-1">
        {users.map((user, index) => (
          <MentionUserItem
            key={user.userId}
            user={user}
            isSelected={index === selectedIndex}
            onSelect={() => onSelect(user)}
            onHover={() => onSelectedIndexChange(index)}
            itemRef={(el) => { itemRefs.current[index] = el; }}
            overrides={overrides}
          />
        ))}
      </div>
    </motion.div>
  );
};

export default MentionAutocomplete;
