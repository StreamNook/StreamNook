// The title bar drops button's hover card: the reward chat's drops lip and
// drawer would show, drawn the same way: its picture, its name, the game and
// channel, an inset track with the percentage, and the minutes.
import type { CurrentDropInfo } from '../../types';
import { DropArt } from '../chat/DropArt';

interface Props {
  drop: CurrentDropInfo;
  channelName?: string;
  percent: number;
}

export function DropsPreviewCard({ drop, channelName, percent }: Props) {
  return (
    <>
      <div className="flex min-w-0 items-center gap-2.5">
        <DropArt image={drop.drop_image ?? ''} size={36} radius={8} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[12.5px] font-medium leading-4 text-textPrimary">{drop.drop_name || '…'}</div>
          <div className="mt-0.5 truncate text-[11px] leading-[14px] text-textSecondary">
            {[drop.game_name, channelName].filter(Boolean).join(' · ')}
          </div>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-2.5">
        <span aria-hidden className="block h-[3px] min-w-0 flex-1 overflow-hidden rounded-full bg-textPrimary/[0.10]">
          <span className="drops-bar-fill block h-full rounded-full" style={{ width: `${percent}%` }} />
        </span>
        <span className="shrink-0 text-[12px] font-semibold tabular-nums text-textPrimary">{percent}%</span>
      </div>
      <div className="mt-1 text-[11px] tabular-nums text-textSecondary">
        {drop.current_minutes} of {drop.required_minutes} min watched
      </div>
      {drop.detail && <div className="mt-0.5 truncate text-[11px] text-textSecondary">{drop.detail}</div>}
      <div className="mt-2.5 border-t border-white/[0.07] pt-2 text-[11px] text-textSecondary">Click for all drops</div>
    </>
  );
}
