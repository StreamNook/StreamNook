// A screen the shell keeps mounted while another tab is in front (Chats), so
// coming back rebuilds nothing. Out of sight it must not paint or take a touch,
// whatever it holds: rooms, the picker, an open sheet.
//
// Hidden by class, not by the `hidden` attribute alone: Tailwind's `[hidden]`
// rule and a display utility such as `flex` weigh the same and utilities come
// later, so `hidden` plus `flex` stays `display: flex` and the whole screen
// painted over the tab in front. The display utility is only there while shown.
import React from 'react';

export function keptSurfaceClass(shown: boolean): string {
  return `absolute inset-0 flex-col ${shown ? 'flex' : 'hidden'}`;
}

export const KeptSurface: React.FC<{
  shown: boolean;
  ref?: React.Ref<HTMLDivElement>;
  style?: React.CSSProperties;
  children: React.ReactNode;
}> = ({ shown, ref, style, children }) => (
  <div
    ref={ref}
    hidden={!shown}
    inert={!shown}
    aria-hidden={shown ? undefined : true}
    className={keptSurfaceClass(shown)}
    style={style}
  >
    {children}
  </div>
);
