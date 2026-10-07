import { useMemo } from 'react';
import { Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { clearUserOverride } from '../../utils/userChatOverrides';
import { SettingsSection } from './_primitives';
import type { UserChatOverride } from '../../types';

const UserOverridesSettings = () => {
  const overrides = useAppStore((s) => s.settings.chat_customization?.user_overrides);

  const entries = useMemo<UserChatOverride[]>(() => {
    if (!overrides) return [];
    return Object.values(overrides).sort((a, b) => {
      const an = a.username ?? a.user_id;
      const bn = b.username ?? b.user_id;
      return an.localeCompare(bn);
    });
  }, [overrides]);

  return (
    <SettingsSection
      label="Nicknames"
      description="Nicknames you've set for individual chatters. Only visible to you. Set or clear a nickname from the user's profile card in chat."
      bare
    >
      {entries.length === 0 ? (
        <div className="bg-glass/30 rounded-lg px-4 py-6 text-center">
          <p className="text-sm text-textSecondary">
            No overrides yet. Click any user's name in chat to open their profile card, then set a nickname.
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          {entries.map((override) => (
            <div
              key={override.user_id}
              className="bg-glass/30 rounded-lg p-3 flex items-center gap-3"
            >
              <div className="flex-1 min-w-0">
                <div className="text-sm text-textPrimary truncate">
                  <span className="font-medium">{override.nickname || '(no nickname)'}</span>
                  <span className="text-textSecondary"> · </span>
                  <span className="text-textSecondary text-xs">@{override.username ?? override.user_id}</span>
                </div>
                {override.color && (
                  <div className="flex items-center gap-1.5 mt-1 text-xs text-textSecondary">
                    <span
                      className="inline-block w-3 h-3 rounded-sm border border-borderSubtle"
                      style={{ backgroundColor: override.color }}
                    />
                    <span className="font-mono">{override.color}</span>
                  </div>
                )}
              </div>
              <button
                onClick={() => clearUserOverride(override.user_id)}
                className="p-1.5 text-textSecondary hover:text-red-400 transition-colors flex-shrink-0"
                aria-label={`Clear override for ${override.username ?? override.user_id}`}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </SettingsSection>
  );
};

export default UserOverridesSettings;
