import type { CSSProperties } from 'react';
import type { ChatEventSettings } from '../../types';

// How event rows dress: the tinted cards, a ring, or nothing, plus the
// optional glint. The glint classes are the same ones first-time rows use
// (see globals.css sn-ft-*). Shared by the chat row and the settings preview.
export const eventCardClass = (events: ChatEventSettings | undefined, gradient: string): string => {
  const style = events?.event_style ?? 'cards';
  const glint = events?.event_animation && events.event_animation !== 'none' ? events.event_animation : null;
  const base = style === 'plain' ? '' : style === 'outline' ? 'sn-event-outline' : gradient;
  const anim = glint
    ? ` ${style === 'outline' ? 'sn-ft-anim-ring' : 'sn-ft-anim-bar'} sn-ft-t-${glint}${events?.event_animate_repeat ? ' sn-ft-loop' : ''}`
    : '';
  return `relative ${base}${anim}`;
};

export const eventCardStyle = (events: ChatEventSettings | undefined): CSSProperties | undefined =>
  (events?.event_style ?? 'cards') === 'outline' && events?.event_outline_color
    ? ({ '--sn-event-outline': events.event_outline_color } as CSSProperties)
    : undefined;
