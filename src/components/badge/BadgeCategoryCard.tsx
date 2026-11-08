import { ArrowUpRight, Gamepad2 } from 'lucide-react';

/** A themed card for the Twitch category a badge is tied to: the game's box art
 *  bleeding to the card edge, the name, and its live viewers as Rust worded
 *  them (services/watch_places.rs). The whole card opens the category. */
export const BadgeCategoryCard = ({
  name,
  boxArtUrl,
  viewersLine,
  onClick,
}: {
  name: string;
  boxArtUrl?: string;
  viewersLine?: string;
  onClick: () => void;
}) => {
  const art = boxArtUrl ? boxArtUrl.replace('{width}', '216').replace('{height}', '288') : null;

  return (
    <button
      onClick={onClick}
      className="group flex items-stretch w-full text-left rounded-xl bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.06] overflow-hidden transition-colors"
    >
      {art ? (
        <img src={art} alt="" className="w-[58px] object-cover shrink-0" />
      ) : (
        <span className="w-[58px] bg-white/[0.06] flex items-center justify-center shrink-0">
          <Gamepad2 size={22} className="text-textMuted" />
        </span>
      )}
      <div className="flex-1 min-w-0 py-2.5 px-3 flex flex-col justify-center">
        <div className="text-[11px] text-textMuted uppercase tracking-wide">Category</div>
        <div className="text-[15px] font-medium text-textPrimary truncate group-hover:text-accent transition-colors">
          {name}
        </div>
        <div className="text-[12px] text-textSecondary">{viewersLine ?? 'Browse this category'}</div>
      </div>
      <ArrowUpRight
        size={18}
        className="text-textMuted group-hover:text-accent transition-colors shrink-0 self-center mr-3"
      />
    </button>
  );
};
