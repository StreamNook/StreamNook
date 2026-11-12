import { useState } from 'react';
import { IS_MOBILE } from '../../utils/platform';
import { Tooltip } from '../ui/Tooltip';
import { orderedSets, setTitle, tileLabel, type SmileyStyle } from './smileyStyle';

interface SmileyStylePickerProps {
  view: SmileyStyle;
  /** The key being switched to, while the change is going out. */
  pending: string | null;
  onSelect: (key: string) => void;
}

/**
 * One tile per smiley set, each showing that set's own `:)` `;)` `<3`, with the
 * picked set ringed in the accent color. A set the account cannot pick sits
 * after the others, dimmed; tapping it opens Twitch's Prime page. Under the
 * tiles, every face of the picked set (on a desktop, the set under the
 * pointer) with the text you type for it.
 */
export const SmileyStylePicker = ({ view, pending, onSelect }: SmileyStylePickerProps) => {
  const [hovered, setHovered] = useState<string | null>(null);
  const phone = IS_MOBILE;
  const size = phone ? 32 : 28;
  const sets = orderedSets(view);
  const shownKey = !phone && hovered ? hovered : view.current;
  const shown = sets.find((s) => s.key === shownKey) ?? sets.find((s) => s.key === view.current) ?? sets[0];

  return (
    <div className="space-y-3">
      <div
        role="radiogroup"
        aria-label="Smiley style"
        aria-busy={pending !== null}
        className="flex flex-wrap gap-2"
        onMouseLeave={phone ? undefined : () => setHovered(null)}
        onBlur={phone ? undefined : () => setHovered(null)}
      >
        {sets.map((set) => {
          const on = set.key === view.current;
          const label = tileLabel(set);
          return (
            <Tooltip key={set.key} content={label} disabled={phone}>
              <button
                type="button"
                role="radio"
                aria-checked={on}
                aria-label={label}
                onClick={() => onSelect(set.key)}
                onMouseEnter={phone ? undefined : () => setHovered(set.key)}
                onFocus={phone ? undefined : () => setHovered(set.key)}
                className={`glass-tile flex items-center gap-1.5 rounded-lg transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70 ${
                  phone ? 'px-3 py-2.5' : 'px-2.5 py-2'
                } ${on ? 'ring-2 ring-inset ring-accent' : ''} ${set.available ? '' : 'opacity-40 grayscale'}`}
              >
                {set.preview.map((face) => (
                  <img
                    key={face.id}
                    src={face.url}
                    alt=""
                    width={size}
                    height={size}
                    draggable={false}
                    className="shrink-0 select-none object-contain"
                  />
                ))}
              </button>
            </Tooltip>
          );
        })}
      </div>
      <div
        role="group"
        aria-label={shown ? `${setTitle(shown)} faces` : 'Faces'}
        data-smiley-faces=""
        className="flex flex-wrap gap-x-3 gap-y-2"
      >
        {(shown?.faces ?? []).map((face) => (
          <div key={face.id} className="flex flex-col items-center gap-1" style={{ minWidth: size + 8 }}>
            <img
              src={face.url}
              alt=""
              width={size}
              height={size}
              draggable={false}
              className="select-none object-contain"
            />
            <span className="text-[11px] leading-none text-textMuted">{face.text}</span>
          </div>
        ))}
      </div>
    </div>
  );
};
