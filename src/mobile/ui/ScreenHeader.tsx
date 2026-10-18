// A tab screen's title row: the large title on the 16px gutter and the screen's
// controls at the trailing edge, in one fixed-height row so every tab puts its
// title on the same baseline. Screens rolled their own rows (Rewards' title sat
// 8dp higher than Following's), and their controls moved between tabs.
import React from 'react';

/** The large title (Brandon: large, like the iPhone): Material 3's large top
 *  app bar headline, 28 bold, the Android step nearest the iPhone's 32/700, so
 *  the screen's name leads the page. */
export const SCREEN_TITLE_CLASS = 'text-[28px] font-bold leading-[34px] tracking-[-0.01em] text-textPrimary';

export const ScreenHeader: React.FC<{
  title: string;
  /** The trailing controls, usually one or two `TitleCapsule`s. The last one's
   *  edge lands on the 16px gutter, the same line the content ends on. */
  children?: React.ReactNode;
}> = ({ title, children }) => (
  <div className="flex h-[60px] shrink-0 items-center justify-between gap-3 px-4">
    <h1 className={`${SCREEN_TITLE_CLASS} min-w-0 truncate`}>{title}</h1>
    {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
  </div>
);
