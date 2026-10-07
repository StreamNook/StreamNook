// Custom sounds: pick an audio file and it becomes a choice in every sound
// picker (highlights, mentions, notifications) under the id `file:<id>`. Rust
// copies the file into the app's own sounds folder, so moving or deleting the
// original never breaks it; playback goes through the asset protocol and
// nothing is uploaded anywhere. Sounds added before imports existed keep
// pointing at the user's own file.

import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Play, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { playSound } from '../../utils/notificationSound';
import { SettingsSection } from './_primitives';
import { Tooltip } from '../ui/Tooltip';
import type { CustomSound } from '../../types';

interface ImportedSound {
  id: string;
  name: string;
  path: string;
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
        filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'oga', 'opus', 'flac', 'm4a', 'aac', 'webm'] }],
      });
      const path = typeof picked === 'string' ? picked : null;
      if (!path) return;
      const imported = await invoke<ImportedSound>('import_custom_sound', { path });
      write([...sounds, { id: `file:${imported.id}`, name: imported.name, path: imported.path }]);
    } catch (err) {
      // A cancelled dialog resolves to null above; anything here is a refusal
      // from the import (type, size) or a dialog that could not open.
      useAppStore.getState().addToast(typeof err === 'string' ? err : 'Could not add that sound.', 'error');
    }
  };

  const remove = (sound: CustomSound) => {
    write(sounds.filter((x) => x.id !== sound.id));
    void invoke('remove_custom_sound', { path: sound.path }).catch(() => {});
  };

  return (
    <SettingsSection
      id="settings-section-custom-sounds"
      label="Custom Sounds"
      description="Your own audio files, up to 2 MB each. Once added, they appear in every sound picker next to the built-in tones: highlights, mentions and notifications. StreamNook keeps its own copy, so moving the original never breaks it."
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
                onClick={() => remove(s)}
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
