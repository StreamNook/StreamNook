import { playSound } from '../../utils/notificationSound';

/** Volume as a percent of the sound's own level, shared by every place a sound
 *  is chosen so every slider means the same thing. Letting go of the slider
 *  plays the sound at the new level. */
export function SoundVolume({
  value,
  onChange,
  sound,
  label,
}: {
  value: number;
  onChange: (next: number) => void;
  sound: string | null | undefined;
  label: string;
}) {
  return (
    <div className="flex items-center gap-2">
      <input
        type="range"
        min={0}
        max={200}
        step={5}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        onPointerUp={() => playSound(sound, value)}
        onKeyUp={() => playSound(sound, value)}
        className="w-32 accent-accent"
        aria-label={label}
      />
      <span className="w-11 text-right text-[12px] tabular-nums text-textMuted">{value}%</span>
    </div>
  );
}
