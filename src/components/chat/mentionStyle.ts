import type { CSSProperties } from 'react';
import type { ChatDesignSettings } from '../../types';

// How an @mention reads inside a message body. One place, shared by the chat
// row (MentionSpan) and the settings preview, so the preview can never drift
// from what chat draws.

export type MentionWeight = 'regular' | 'medium' | 'bold';
export type MentionItalic = 'inherit' | 'never' | 'always';
export type MentionShape = 'plain' | 'pill';

export interface MentionLook {
  weight: MentionWeight;
  italic: MentionItalic;
  shape: MentionShape;
  /** A fixed colour, or null to follow the mentioned user's name colour. */
  color: string | null;
}

export function mentionLook(design: Partial<ChatDesignSettings> | undefined | null): MentionLook {
  return {
    weight: design?.mention_weight ?? 'medium',
    italic: design?.mention_italic ?? 'inherit',
    shape: design?.mention_style ?? 'plain',
    color: design?.mention_text_color || null,
  };
}

const WEIGHT_CLASS: Record<MentionWeight, string> = {
  regular: 'font-normal',
  medium: 'font-medium',
  bold: 'font-bold',
};

const ITALIC_CLASS: Record<MentionItalic, string> = {
  // A /me message is italic as a whole; inherit lets a mention inside it match.
  inherit: '',
  never: 'not-italic',
  always: 'italic',
};

/** Classes for the mention's outer box: spacing, weight and slant. */
export function mentionBoxClass(look: MentionLook): string {
  return ['inline-block px-1.5 py-0.5 rounded', WEIGHT_CLASS[look.weight], ITALIC_CLASS[look.italic]]
    .filter(Boolean)
    .join(' ');
}

/** The outer box's fill. The pill tints with the mention's colour; it sits on
 *  the outer box because a 7TV paint draws the text itself through
 *  `background-clip: text` on the inner span, and one element cannot carry
 *  both backgrounds. */
export function mentionBoxStyle(look: MentionLook, baseColor: string): CSSProperties | undefined {
  if (look.shape !== 'pill') return undefined;
  return { backgroundColor: `color-mix(in srgb, ${look.color ?? baseColor} 16%, transparent)` };
}

/** The text's own style: a fixed colour when one is chosen, otherwise the
 *  user's name colour or paint as resolved by the caller. */
export function mentionTextStyle(look: MentionLook, nameStyle: CSSProperties): CSSProperties {
  return look.color ? { color: look.color } : nameStyle;
}
