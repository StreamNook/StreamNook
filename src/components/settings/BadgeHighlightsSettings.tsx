import { EntryInput } from './entryInput';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { SettingsSection } from './_primitives';
import { Tooltip } from '../ui/Tooltip';
import type { HighlightBadge } from '../../types';

/** One edit to the stored badge highlights; Rust makes new entries (a seed
 *  item is merged over the default) and trims badge keys. Answers the list as
 *  stored, or null when the edit failed. */
async function editBadges(edit: Record<string, unknown>): Promise<HighlightBadge[] | null> {
  try {
    return await invoke<HighlightBadge[]>('edit_settings_list', { path: 'chat_highlights.badges', edit });
  } catch (err) {
    Logger.warn('[BadgeHighlights] edit failed:', err);
    return null;
  }
}

// Quick-add presets — saves the user from typing exact badge keys.
const BADGE_PRESETS: Array<{ label: string; key: string; color: string }> = [
  { label: 'Broadcaster', key: 'broadcaster/1', color: '#ef4444' },
  { label: 'Moderator', key: 'moderator/1', color: '#22c55e' },
  { label: 'VIP', key: 'vip/1', color: '#ec4899' },
  { label: 'Any subscriber', key: 'subscriber/*', color: '#a855f7' },
  { label: 'Twitch staff', key: 'staff/1', color: '#f59e0b' },
  { label: 'Twitch admin', key: 'admin/1', color: '#f97316' },
  { label: 'Partner', key: 'partner/1', color: '#06b6d4' },
];

const BadgeHighlightsSettings = () => {
  const badges: HighlightBadge[] = useAppStore((s) => s.settings.chat_highlights?.badges) ?? [];

  const addBadge = (preset?: typeof BADGE_PRESETS[number]) =>
    void editBadges({
      op: 'add',
      ...(preset ? { item: { badge_key: preset.key, color: preset.color, label: preset.label } } : {}),
    });

  const patchBadge = (id: string, patch: Partial<HighlightBadge>) => void editBadges({ op: 'update', id, patch });

  const saveField = (id: string, field: 'badge_key' | 'color') => async (text: string) => {
    const stored = await editBadges({ op: 'update', id, patch: { [field]: text } });
    return stored?.find((b) => b.id === id)?.[field];
  };

  const removeBadge = (id: string) => void editBadges({ op: 'remove', id });

  return (
    <SettingsSection
      label="Badge Highlights"
      description="Highlight every message from users carrying a specific Twitch badge. Use name/version (e.g. moderator/1) or name/* to match any version."
      bare
    >
      <div className="space-y-3">
        {/* Quick presets */}
        <div className="flex flex-wrap gap-1.5">
          {BADGE_PRESETS.map((p) => {
            const already = badges.some((b) => b.badge_key.toLowerCase() === p.key.toLowerCase());
            return (
              <Tooltip key={p.key} content={p.key}>
              <button
                onClick={() => addBadge(p)}
                disabled={already}
                className="glass-button-secondary text-[11px] px-2 py-1 rounded-md text-textSecondary hover:text-textPrimary disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
              >
                <Plus size={10} className="inline mr-1 -mt-0.5" />
                {p.label}
              </button>
              </Tooltip>
            );
          })}
        </div>

        {badges.length === 0 && (
          <div className="bg-glass/30 rounded-lg px-4 py-6 text-center">
            <p className="text-sm text-textSecondary mb-3">No badge highlights yet.</p>
            <button
              onClick={() => addBadge()}
              className="glass-button inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-textPrimary text-sm font-medium"
            >
              <Plus size={14} />
              Add a custom badge
            </button>
          </div>
        )}

        {badges.map((b) => (
          <div key={b.id} className="bg-glass/30 rounded-lg p-3">
            <div className="flex items-center gap-2">
              <button
                onClick={() => patchBadge(b.id, { enabled: !b.enabled })}
                className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors flex-shrink-0 ${
                  b.enabled ? 'bg-accent' : 'bg-gray-600'
                }`}
                aria-label={b.enabled ? 'Disable badge highlight' : 'Enable badge highlight'}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                    b.enabled ? 'translate-x-5' : 'translate-x-1'
                  }`}
                />
              </button>

              <EntryInput
                type="text"
                value={b.badge_key}
                save={saveField(b.id, 'badge_key')}
                placeholder="moderator/1 or subscriber/*"
                className="flex-1 glass-input text-textPrimary text-sm px-2.5 py-1.5 font-mono"
                spellCheck={false}
              />

              <EntryInput
                type="color"
                value={b.color}
                save={saveField(b.id, 'color')}
                className="w-8 h-8 rounded cursor-pointer bg-transparent border border-borderSubtle"
                aria-label="Highlight color"
              />

              <button
                onClick={() => removeBadge(b.id)}
                className="p-1 text-textSecondary hover:text-red-400 transition-colors"
                aria-label="Delete badge highlight"
              >
                <Trash2 size={14} />
              </button>
            </div>

            <div className="flex items-center gap-4 pl-11 pt-2 text-xs text-textSecondary">
              <label className="inline-flex items-center gap-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={b.hold === true}
                  onChange={(e) => patchBadge(b.id, { hold: e.target.checked })}
                  className="accent-accent"
                />
                Keep tinted
              </label>
            </div>
          </div>
        ))}

        {badges.length > 0 && (
          <button
            onClick={() => addBadge()}
            className="inline-flex items-center gap-2 px-3 py-1.5 text-textSecondary hover:text-textPrimary text-sm transition-colors"
          >
            <Plus size={14} />
            Add custom badge
          </button>
        )}
      </div>
    </SettingsSection>
  );
};

export default BadgeHighlightsSettings;
