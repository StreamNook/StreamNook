// The phone's grouped list: a quiet label over a group of rows, each row a
// leading icon then its label, and a quiet line under the group. The chat menu
// and the settings pages are both built from these pieces, so a row reads the
// same wherever it appears and the two can never drift apart.
//
// A group sits on one of two surfaces. `card` is a solid card of its own, for
// a page (the settings pages): opaque at every Glassiness, the way a static
// popover is, so nothing behind it ever reads through. `sheet` draws the rows
// straight on a surface that is already solid enough to read on (a bottom
// sheet), with no card and no hairlines, which is how the chat menu reads.
import React from 'react';
import { GroupSurfaceContext, useGroupRowFrame, useGroupSurface, type GroupSurface } from './groupSurface';

/** The glyph size inside the icon column. Lucide glyphs pair it with
 *  GROUP_ICON_STROKE, which draws them at the weight of the chat menu's
 *  phosphor outlines, so the two icon sets sit side by side evenly. */
export const GROUP_ICON = 22;
export const GROUP_ICON_STROKE = 1.5;

/** The icon column every row shares: a 24px box with the glyph centered. */
export const GroupIcon: React.FC<{ children?: React.ReactNode; className?: string }> = ({
  children,
  className = '',
}) => (
  <span className={`flex w-6 h-6 shrink-0 items-center justify-center ${className}`} aria-hidden>
    {children}
  </span>
);

type LabelProps = React.HTMLAttributes<HTMLElement> & {
  /** A section heading on a page is an h3; a label inside a sheet is a div. */
  as?: 'div' | 'h3';
};

/** The quiet label over a group. */
export const GroupLabel: React.FC<LabelProps> = ({ as = 'div', className = '', children, ...rest }) => {
  const Tag = as;
  return (
    <Tag
      className={`text-[12px] font-semibold uppercase tracking-wide text-textMuted ${className}`}
      {...rest}
    >
      {children}
    </Tag>
  );
};

/** The quiet line under a group: what the section covers, or a note on what
 *  the rows above it do. */
export const GroupFooter: React.FC<{ children: React.ReactNode; className?: string }> = ({
  children,
  className = '',
}) => <div className={`text-[12px] leading-snug text-textMuted ${className}`}>{children}</div>;

/** A group of rows. On a card the rows run edge to edge inside it (they bleed
 *  over its 16px padding), so their hairlines can reach the trailing edge;
 *  anything else inside keeps the padding. */
export const GroupCard: React.FC<{
  children: React.ReactNode;
  surface?: GroupSurface;
  className?: string;
}> = ({ children, surface = 'card', className = '' }) => (
  <GroupSurfaceContext.Provider value={surface}>
    {surface === 'card' ? (
      <div className={`sn-group-card px-4 ${className}`}>
        <div className="sn-group-body">{children}</div>
      </div>
    ) : (
      <div className={`flex flex-col ${className}`}>{children}</div>
    )}
  </GroupSurfaceContext.Provider>
);

type RowProps = Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'children'> & {
  /** The leading glyph, drawn in the icon column. */
  icon: React.ReactNode;
  /** The row's name. */
  children: React.ReactNode;
  /** A quiet line under the name. */
  detail?: React.ReactNode;
  /** A quiet word at the trailing edge ("On", "Subscribed"). */
  value?: React.ReactNode;
  /** Anything else at the trailing edge (a chevron). */
  trailing?: React.ReactNode;
};

/** An action row: a leading icon, then the name, then an optional value or
 *  chevron at the trailing edge. 44px tall at least, a finger's reach. */
export const GroupRow: React.FC<RowProps> = ({
  icon,
  children,
  detail,
  value,
  trailing,
  className = '',
  type = 'button',
  ...rest
}) => {
  const frame = useGroupRowFrame();
  return (
    <button
      type={type}
      className={`sn-touch ${frame} flex items-center gap-3 text-[15px] text-textPrimary active:opacity-70 disabled:opacity-45 ${
        detail ? 'py-2.5' : ''
      } ${className}`}
      {...rest}
    >
      <GroupIcon className={detail ? 'self-start' : ''}>{icon}</GroupIcon>
      {detail ? (
        <span className="flex-1 min-w-0 text-left">
          <span className="block leading-6">{children}</span>
          <span className="block text-[13px] leading-[18px] text-textMuted">{detail}</span>
        </span>
      ) : (
        <span className="flex-1 text-left">{children}</span>
      )}
      {value !== undefined && value !== null && <span className="text-[12px] text-textMuted">{value}</span>}
      {trailing}
    </button>
  );
};

/** The break before the rows that take something away. */
export const GroupDivider: React.FC = () => {
  const surface = useGroupSurface();
  return <div className={`my-1 border-t border-borderSubtle ${surface === 'card' ? '-mx-4' : 'mx-2'}`} />;
};
