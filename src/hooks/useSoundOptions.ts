import { useMemo } from 'react';
import { useAppStore } from '../stores/AppStore';
import { SOUND_LABELS, type SoundId } from '../utils/notificationSound';
import type { CustomSound } from '../types';

const BUILT_INS: SoundId[] = ['boop', 'tick', 'soft', 'whisper', 'gentle'];
const NO_CUSTOM: CustomSound[] = [];

/** A sound picker's choices: the built-in tones, then the viewer's own sounds,
 *  so custom sounds show up in every picker. `noneLabel` adds an "off" choice
 *  whose value is the empty string. */
export function useSoundOptions({ defaultId, noneLabel }: { defaultId?: SoundId; noneLabel?: string } = {}) {
  const custom = useAppStore((s) => s.settings.chat_highlights?.custom_sounds ?? NO_CUSTOM);
  return useMemo(
    () => [
      ...(noneLabel ? [{ value: '', label: noneLabel }] : []),
      ...BUILT_INS.map((id) => ({ value: id, label: id === defaultId ? `${SOUND_LABELS[id]} (Default)` : SOUND_LABELS[id] })),
      ...custom.map((c) => ({ value: c.id, label: c.name || 'Custom sound' })),
    ],
    [custom, defaultId, noneLabel],
  );
}
