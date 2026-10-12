import { MousePointer2 } from 'lucide-react';
import { SettingPreview } from './ChatPreview';

/**
 * A tiny window on loop: the pointer climbs to the top edge, the title bar
 * slides down over the picture, the pointer leaves and the bar slides away
 * again. Transform and opacity only, so it never repaints; with motion off or
 * reduced it holds still on the revealed frame (globals.css, `.sn-tbt-*`).
 */
export const TitleBarTuckPreview = () => (
  <SettingPreview caption="the bar comes back when the pointer reaches the top">
    <div
      aria-hidden
      className="sn-tbt-window relative h-[96px] w-[170px] overflow-hidden rounded-md border border-borderSubtle bg-background"
    >
      {/* The picture and the chat both run to the top edge: nothing reserves
          the bar's row. */}
      <div className="flex h-full w-full">
        <div className="sn-tbt-picture h-full flex-1" />
        <div className="flex h-full w-[38px] flex-col gap-[5px] border-l border-borderSubtle bg-background px-[5px] pt-[6px]">
          <span className="h-[3px] w-[22px] rounded-full bg-textMuted" />
          <span className="h-[3px] w-[26px] rounded-full bg-textMuted" />
          <span className="h-[3px] w-[18px] rounded-full bg-textMuted" />
          <span className="h-[3px] w-[24px] rounded-full bg-textMuted" />
          <span className="h-[3px] w-[20px] rounded-full bg-textMuted" />
        </div>
      </div>
      <div className="sn-tbt-bar absolute inset-x-0 top-0 flex h-[15px] items-center justify-between px-1.5">
        <div className="flex items-center gap-1">
          <span className="h-[7px] w-[7px] rounded-full bg-textPrimary" />
          <span className="h-[6px] w-6 rounded-full bg-textSecondary" />
        </div>
        <span className="h-[6px] w-9 rounded-full bg-textSecondary" />
        <div className="flex items-center gap-[3px]">
          <span className="h-[5px] w-[5px] rounded-full bg-textSecondary" />
          <span className="h-[5px] w-[5px] rounded-full bg-textSecondary" />
          <span className="h-[5px] w-[5px] rounded-full bg-textSecondary" />
        </div>
      </div>
      <MousePointer2
        size={11}
        className="sn-tbt-pointer pointer-events-none absolute left-0 top-0 fill-white text-black"
      />
    </div>
  </SettingPreview>
);
