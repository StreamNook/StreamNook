import { ArrowSquareOut } from 'phosphor-react';
import { openExternal } from '../../utils/openExternal';
import { GLAZE_PILL } from '../ui/glazeButtons';

/** The game account is not linked, so Twitch will not grant the reward: the
 *  game's page to link it, in the browser. Twitch's own Inventory offers the
 *  same in place of Claim. Its own file so the bar's hover card can draw it
 *  without loading the drawer. */
export function ConnectButton({ url, className = '' }: { url: string; className?: string }) {
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        void openExternal(url);
      }}
      className={`${GLAZE_PILL} inline-flex items-center justify-center gap-1 px-3 text-[11.5px] leading-[22px] ${className}`}
    >
      Connect
      <ArrowSquareOut size={11} weight="bold" aria-hidden className="opacity-70" />
    </button>
  );
}
