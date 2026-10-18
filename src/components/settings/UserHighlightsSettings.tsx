import { EntryInput } from './entryInput';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { SettingsSection } from './_primitives';
import type { HighlightUser } from '../../types';

/** One edit to the stored user highlights; Rust makes new entries and stores
 *  logins trimmed and lowercased. Answers the list as stored, or null when
 *  the edit failed. */
async function editUsers(edit: Record<string, unknown>): Promise<HighlightUser[] | null> {
  try {
    return await invoke<HighlightUser[]>('edit_settings_list', { path: 'chat_highlights.users', edit });
  } catch (err) {
    Logger.warn('[UserHighlights] edit failed:', err);
    return null;
  }
}

const UserHighlightsSettings = () => {
  const users: HighlightUser[] = useAppStore((s) => s.settings.chat_highlights?.users) ?? [];

  const addUser = () => void editUsers({ op: 'add' });

  const patchUser = (id: string, patch: Partial<HighlightUser>) => void editUsers({ op: 'update', id, patch });

  const saveField = (id: string, field: 'username' | 'color') => async (text: string) => {
    const stored = await editUsers({ op: 'update', id, patch: { [field]: text } });
    return stored?.find((u) => u.id === id)?.[field];
  };

  const removeUser = (id: string) => void editUsers({ op: 'remove', id });

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

              <EntryInput
                type="text"
                value={u.username}
                save={saveField(u.id, 'username')}
                placeholder="twitch_login"
                className="flex-1 glass-input text-textPrimary text-sm px-2.5 py-1.5"
                spellCheck={false}
              />

              <EntryInput
                type="color"
                value={u.color}
                save={saveField(u.id, 'color')}
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
