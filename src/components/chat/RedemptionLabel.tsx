import type { ReactNode } from 'react';

/** The line over a channel points row: the points glyph and what was redeemed,
 *  right-aligned, in the row's Redemption color (`sn-redeem-ink`, the theme's
 *  cyan until one is picked). `children` follows the label (the reward's
 *  cost). Shared by the chat row and the Redemption style preview. */
export const RedemptionLabel = ({ label, children }: { label: string; children?: ReactNode }) => (
  <div className="flex min-w-0 items-center justify-end gap-1">
    <svg className="sn-redeem-ink w-3 h-3 flex-shrink-0 opacity-60" viewBox="0 0 20 20" fill="currentColor">
      <path d="M10 6a4 4 0 014 4h-2a2 2 0 00-2-2V6z" />
      <path fillRule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-2 0a6 6 0 11-12 0 6 6 0 0112 0z" clipRule="evenodd" />
    </svg>
    <span className="sn-redeem-ink min-w-0 truncate text-xs font-normal opacity-60">{label}</span>
    {children}
  </div>
);
