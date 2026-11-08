import { X } from 'phosphor-react';
import { getAppleEmojiUrl } from '../../services/emojiService';
import { Tooltip } from '../ui/Tooltip';

/** The emote button inset at the message box's left: the cycling smiley, or
 *  a close mark while the picker is open. Chat's composer and Settings >
 *  Chat's preview both draw it. */
export function EmoteSmileyButton({
  open,
  smiley,
  transitioning,
  label,
  buttonRef,
  onClick,
  onMouseLeave,
}: {
  open: boolean;
  smiley: string;
  transitioning: boolean;
  label: string;
  buttonRef?: React.Ref<HTMLButtonElement>;
  onClick?: () => void;
  onMouseLeave?: () => void;
}) {
  return (
    <Tooltip content={open ? 'Close' : label} side="top">
      <button
        ref={buttonRef}
        onClick={onClick}
        onMouseLeave={onMouseLeave}
        className="group absolute left-1 top-1/2 -translate-y-1/2 z-10 flex items-center justify-center w-7 h-7 text-textSecondary hover:text-textPrimary transition-colors duration-200"
      >
        {open ? (
          <X size={16} weight="bold" aria-hidden className="transition-all duration-200 text-accent group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)]" />
        ) : (
          <img
            src={getAppleEmojiUrl(smiley)}
            alt={smiley}
            draggable={false}
            className={`w-4 h-4 object-contain transition-all ease-in-out group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)] ${
              transitioning ? 'opacity-0 scale-50 duration-100' : 'opacity-100 scale-100 duration-150'
            }`}
          />
        )}
      </button>
    </Tooltip>
  );
}
