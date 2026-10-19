// Quality picker as a bottom sheet. Same store round-trip as the desktop menu:
// AppStore.availableQualities -> changeStreamQuality (the backend restarts the
// loopback stream at the chosen quality).
import React, { useState } from 'react';
import { Check } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { MobileSheet } from '../ui/MobileSheet';
import { SleepTimerSection } from './SleepTimerSection';

// Row names, the same words the iPhone sheet uses. Labelled here rather than
// with CSS `capitalize`, which read the raw keys back as "Audio_only" and
// "Worst". A named tier keeps its own spelling past the first letter, so
// "1080p60 (source)" is not turned into "1080p60 (Source)".
const qualityLabel = (q: string): string => {
  switch (q) {
    case 'best':
      return 'Auto';
    case 'audio_only':
      return 'Audio only';
    case 'worst':
      return 'Lowest';
    default:
      return q.charAt(0).toUpperCase() + q.slice(1);
  }
};

// Auto first (the way back to the screen cap), then the core's tiers in the
// order Rust gave them.
const qualityRows = (available: string[]): string[] => [
  ...available.filter((q) => q === 'best'),
  ...available.filter((q) => q !== 'best'),
];

export const QualitySheet: React.FC<{ open: boolean; onClose: () => void }> = ({
  open,
  onClose,
}) => {
  const availableQualities = useAppStore((s) => s.availableQualities);
  const changeStreamQuality = useAppStore((s) => s.changeStreamQuality);
  const savedQuality = useAppStore((s) => s.settings.quality);
  const [selected, setSelected] = useState<string | null>(null);

  // What is actually in effect: this session's pick if there was one, else the
  // saved setting. Previously nothing was ticked until you picked something,
  // which made it impossible to see what you were already on.
  const current = selected ?? savedQuality ?? 'best';

  const pick = (quality: string) => {
    setSelected(quality);
    onClose();
    void changeStreamQuality(quality);
  };

  return (
    <MobileSheet open={open} onClose={onClose} title="Quality">
      {availableQualities.length === 0 ? (
        <div className="py-6 text-center text-sm text-textMuted">
          Quality options appear once the stream loads.
        </div>
      ) : (
        <div className="flex flex-col">
          {qualityRows(availableQualities).map((q) => {
            // "best" is shown as Auto on a phone, because it no longer means
            // "the highest tier that exists" - it means the highest tier this
            // SCREEN can show. Calling that "best" would read as a downgrade to
            // anyone who noticed the resolution drop.
            //
            // Keeping it labelled and pickable is the point, not decoration: a
            // named tier is written to settings and persists across launches and
            // across channels, so without an obvious way back, one curious tap
            // on 1440p60 would silently disable the screen-size cap for good.
            const isAuto = q === 'best';
            return (
              <button
                key={q}
                onClick={() => pick(q)}
                className="sn-touch flex items-center justify-between px-2 text-[15px] text-textPrimary active:opacity-70"
              >
                <span className="flex flex-col items-start leading-tight">
                  <span>{qualityLabel(q)}</span>
                  {isAuto && (
                    <span className="text-[11px] text-textMuted">Matches your screen</span>
                  )}
                </span>
                {/* The tick matches the label's size, never bigger than the
                    words it marks. */}
                {current === q && <Check size={15} className="text-accent shrink-0" />}
              </button>
            );
          })}
        </div>
      )}
      <SleepTimerSection />
    </MobileSheet>
  );
};
