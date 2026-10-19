import { Loader2 } from 'lucide-react';
import { TwitchGlyph } from './TwitchGlyph';

/**
 * The Twitch sign-in: Twitch's purple with its glyph, a spinner in the
 * glyph's place while it waits. One look wherever a phone asks for the
 * sign-in, the setup wizard's Twitch step and the signed-out screen alike.
 */
export function TwitchSignInButton({
  label,
  onClick,
  busy = false,
  disabled,
  className = '',
}: {
  label: string;
  onClick: () => void;
  busy?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled ?? busy}
      className={`flex items-center justify-center gap-2 rounded-lg font-medium bg-[#9146FF] text-white active:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed ${className}`}
    >
      {busy ? <Loader2 size={18} className="animate-spin" /> : <TwitchGlyph size={18} />}
      {label}
    </button>
  );
}
