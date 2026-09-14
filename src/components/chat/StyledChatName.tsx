import { type CSSProperties, type ReactNode, type MouseEventHandler } from 'react';
import { Tooltip } from '../ui/Tooltip';

export type NameSeparator = 'none' | 'colon' | 'dot' | 'arrow' | 'pipe' | 'dash';
export type NameStyle = 'plain' | 'bar' | 'chip' | 'brackets' | 'dot';

// Glyph drawn between the username and the message body. 'none' renders nothing.
// Dash is an en dash, never an em dash.
const SEPARATOR_GLYPHS: Record<NameSeparator, string> = {
  none: '',
  colon: ':',
  dot: '·',
  arrow: '›',
  pipe: '|',
  dash: '–',
};

interface StyledChatNameProps {
  name: string;
  /** Style applied to the name text: a plain { color } or a computed 7TV paint. */
  nameTextStyle: CSSProperties;
  nameStyle: NameStyle;
  separator: NameSeparator;
  /** Color for the separator glyph, accent bar, dot, brackets, and chip tint. */
  accentColor: string;
  /** Extra node rendered inside the name span (e.g. the partner verified badge). */
  badge?: ReactNode;
  /** Chat mode wraps the name in the reply tooltip + makes it clickable. The
   *  settings preview passes this falsey so the name is inert. */
  interactive?: boolean;
  onClick?: MouseEventHandler<HTMLSpanElement>;
  onContextMenu?: MouseEventHandler<HTMLSpanElement>;
}

/**
 * The styled chat username: an optional accent bar / color dot before the name,
 * the name itself (optionally a frosted chip or bracketed), and an optional
 * trailing separator glyph. Shared by the live chat row (ChatMessage) and the
 * Chat-settings preview so the two never drift. Applies to normal messages only;
 * action ("/me") messages render their name plain elsewhere.
 */
export function StyledChatName({
  name,
  nameTextStyle,
  nameStyle,
  separator,
  accentColor,
  badge,
  interactive = false,
  onClick,
  onContextMenu,
}: StyledChatNameProps) {
  const glyph = SEPARATOR_GLYPHS[separator];
  const isChip = nameStyle === 'chip';

  const nameSpan = (
    <span
      style={{
        ...nameTextStyle,
        fontWeight: 700,
        // Chip uses backgroundColor (not the `background` shorthand) so it never
        // clobbers a 7TV paint's gradient; the inset bevel is glow-free.
        ...(isChip
          ? {
              padding: '0.05em 0.45em',
              borderRadius: '0.5em',
              backgroundColor: `color-mix(in srgb, ${accentColor} 16%, transparent)`,
              boxShadow: `inset 0 0 0 1px color-mix(in srgb, ${accentColor} 34%, transparent)`,
            }
          : {}),
      }}
      // inline-block, not inline-flex: a flex container with centered items
      // gets a synthesized baseline, floating the name above the message text.
      //
      // But inline-block is also ATOMIC — it cannot break internally. A long
      // handle (YouTube's are unbounded, and the @ prefix adds to it) became one
      // unbreakable box that couldn't fit beside the message start, so the whole
      // name wrapped onto its own line and pushed the body down with it.
      // `max-w-full` + anywhere-wrapping lets the name break inside itself
      // instead, so the message keeps flowing next to it. Short names are
      // unaffected: there is nothing to break.
      className={`inline-block max-w-full [overflow-wrap:anywhere] ${interactive ? 'cursor-pointer' : ''} ${interactive && !isChip ? 'hover:underline' : ''}`}
      onClick={onClick}
      onContextMenu={onContextMenu}
      data-no-drag="true"
    >
      {nameStyle === 'brackets' && <span style={{ color: accentColor }}>[</span>}
      {name}
      {badge}
      {nameStyle === 'brackets' && <span style={{ color: accentColor }}>]</span>}
    </span>
  );

  return (
    <>
      {nameStyle === 'bar' && (
        <span
          aria-hidden="true"
          className="inline-block flex-shrink-0"
          style={{ width: '2px', height: '0.95em', borderRadius: '1px', backgroundColor: accentColor, marginRight: '0.4em', verticalAlign: '-0.12em' }}
        />
      )}
      {nameStyle === 'dot' && (
        <span
          aria-hidden="true"
          className="inline-block flex-shrink-0"
          style={{ width: '0.45em', height: '0.45em', borderRadius: '9999px', backgroundColor: accentColor, marginRight: '0.4em', verticalAlign: '0.05em' }}
        />
      )}
      {interactive ? (
        <Tooltip content="Right-click to reply" side="top">
          {nameSpan}
        </Tooltip>
      ) : (
        nameSpan
      )}
      {glyph && (
        <span
          aria-hidden="true"
          style={{ color: accentColor, fontWeight: 700, marginLeft: separator === 'colon' ? 0 : '0.35em' }}
        >
          {glyph}
        </span>
      )}
    </>
  );
}
