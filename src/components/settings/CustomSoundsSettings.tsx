// Custom highlight sounds: pick any audio file on disk; it becomes a choice in
// every highlight sound dropdown (phrases, users, badges) under the id
// `file:<random>`. The path is stored in settings; playback goes through the
// asset protocol, so nothing is copied or uploaded anywhere.

import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Play, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { playSound } from '../../utils/notificationSound';
import { SettingsSection } from './_primitives';
import { Tooltip } from '../ui/Tooltip';
import type { CustomSound } from '../../types';

function newId(): string {
  const rand =
    typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10);
  return `file:${rand}`;
}

const CustomSoundsSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const sounds = settings.chat_highlights?.custom_sounds ?? [];

  const write = (next: CustomSound[]) =>
    updateSettings({
      ...settings,
      chat_highlights: { ...settings.chat_highlights, phrases: settings.chat_highlights?.phrases ?? [], custom_sounds: next },
    });

  const add = async () => {
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'm4a', 'aac', 'webm'] }],
      });
      const path = typeof picked === 'string' ? picked : null;
      if (!path) return;
      const name = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, '') || 'Sound';
      write([...sounds, { id: newId(), name, path }]);
    } catch {
      /* dialog cancelled or unavailable */
    }
  };

  return (
    <SettingsSection
      id="settings-section-custom-sounds"
      label="Custom Sounds"
      description="Your own audio files as highlight sounds. Once added, they appear in every highlight sound picker next to the built-in tones. Files stay where they are; nothing is copied."
      bare
    >
      <div className="space-y-2">
        {sounds.map((s) => (
          <div key={s.id} className="glass-tile flex items-center gap-2 px-3 py-2">
            <div className="min-w-0 flex-1">
              <input
                type="text"
                value={s.name}
                onChange={(e) => write(sounds.map((x) => (x.id === s.id ? { ...x, name: e.target.value } : x)))}
                className="w-full bg-transparent text-sm font-medium text-textPrimary outline-none"
                spellCheck={false}
                aria-label="Sound name"
              />
              <div className="truncate text-[11px] text-textSecondary" title={s.path}>
                {s.path}
              </div>
            </div>
            <Tooltip content="Play" side="top">
              <button
                type="button"
                onClick={() => playSound(s.id)}
                className="glass-button grid h-7 w-7 place-items-center text-textSecondary hover:text-textPrimary"
                aria-label="Play sound"
              >
                <Play size={13} />
              </button>
            </Tooltip>
            <Tooltip content="Remove" side="top">
              <button
                type="button"
                onClick={() => write(sounds.filter((x) => x.id !== s.id))}
                className="glass-button grid h-7 w-7 place-items-center text-textSecondary hover:text-error"
                aria-label="Remove sound"
              >
                <Trash2 size={13} />
              </button>
            </Tooltip>
          </div>
        ))}
        <button
          type="button"
          onClick={() => void add()}
          className="glass-button inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium text-textPrimary"
        >
          <FolderOpen size={14} />
          Add sound file
        </button>
      </div>
    </SettingsSection>
  );
};

export default CustomSoundsSettings;
