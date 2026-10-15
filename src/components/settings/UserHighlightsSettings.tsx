import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection } from './_primitives';
import type { HighlightUser } from '../../types';

function newRuleId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `u-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

const UserHighlightsSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const users: HighlightUser[] = settings.chat_highlights?.users ?? [];

  const writeUsers = (next: HighlightUser[]) =>
    updateSettings({
      ...settings,
      chat_highlights: {
        phrases: settings.chat_highlights?.phrases ?? [],
        ...settings.chat_highlights,
        users: next,
      },
    });

  const addUser = () =>
    writeUsers([
      ...users,
      { id: newRuleId(), enabled: true, username: '', color: '#22d3ee' },
    ]);

  const patchUser = (id: string, patch: Partial<HighlightUser>) =>
    writeUsers(users.map((u) => (u.id === id ? { ...u, ...patch } : u)));

  const removeUser = (id: string) => writeUsers(users.filter((u) => u.id !== id));

  return (
    <SettingsSection
      label="Username Highlights"
      description="Always highlight messages from specific users by login. Match is case-insensitive."
      bare
    >
      <div className="space-y-3">
        {users.length === 0 && (
          <div className="bg-glass/30 rounded-lg px-4 py-6 text-center">
            <p className="text-sm text-textSecondary mb-3">No user highlights yet.</p>
            <button
              onClick={addUser}
              className="glass-button inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-textPrimary text-sm font-medium"
            >
              <Plus size={14} />
              Add your first user
            </button>
          </div>
        )}

        {users.map((u) => (
          <div key={u.id} className="bg-glass/30 rounded-lg p-3">
            <div className="flex items-center gap-2">
              <button
                onClick={() => patchUser(u.id, { enabled: !u.enabled })}
                className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors flex-shrink-0 ${
                  u.enabled ? 'bg-accent' : 'bg-gray-600'
                }`}
                aria-label={u.enabled ? 'Disable user highlight' : 'Enable user highlight'}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                    u.enabled ? 'translate-x-5' : 'translate-x-1'
                  }`}
                />
              </button>

              <input
                type="text"
                value={u.username}
                onChange={(e) => patchUser(u.id, { username: e.target.value.toLowerCase().trim() })}
                placeholder="twitch_login"
                className="flex-1 glass-input text-textPrimary text-sm px-2.5 py-1.5"
                spellCheck={false}
              />

              <input
                type="color"
                value={u.color}
                onChange={(e) => patchUser(u.id, { color: e.target.value })}
                className="w-8 h-8 rounded cursor-pointer bg-transparent border border-borderSubtle"
                aria-label="Highlight color"
              />

              <button
                onClick={() => removeUser(u.id)}
                className="p-1 text-textSecondary hover:text-red-400 transition-colors"
                aria-label="Delete user highlight"
              >
                <Trash2 size={14} />
              </button>
            </div>

            <div className="flex items-center gap-4 pl-11 pt-2 text-xs text-textSecondary">
              <label className="inline-flex items-center gap-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={u.hold === true}
                  onChange={(e) => patchUser(u.id, { hold: e.target.checked })}
                  className="accent-accent"
                />
                Keep tinted
              </label>
            </div>
          </div>
        ))}

        {users.length > 0 && (
          <button
            onClick={addUser}
            className="inline-flex items-center gap-2 px-3 py-1.5 text-textSecondary hover:text-textPrimary text-sm transition-colors"
          >
            <Plus size={14} />
            Add another user
          </button>
        )}
      </div>
    </SettingsSection>
  );
};

export default UserHighlightsSettings;
