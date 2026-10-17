// A link to a page outside the app. It opens in the viewer's browser through
// `openExternal` on a click or a middle-click, and never leans on the
// webview's own handling of `target="_blank"`, which is not something every
// window and platform can be counted on for.
import React from 'react';
import { openExternal } from '../../utils/openExternal';

type Props = Omit<React.AnchorHTMLAttributes<HTMLAnchorElement>, 'target' | 'rel'> & { href: string };

export const ExternalAnchor = React.forwardRef<HTMLAnchorElement, Props>(function ExternalAnchor(
  { href, onClick, onAuxClick, children, ...rest },
  ref,
) {
  return (
    <a
      ref={ref}
      href={href}
      {...rest}
      onClick={(e) => {
        onClick?.(e);
        if (e.defaultPrevented) return;
        e.preventDefault();
        void openExternal(href);
      }}
      onAuxClick={(e) => {
        onAuxClick?.(e);
        if (e.defaultPrevented || e.button !== 1) return;
        e.preventDefault();
        void openExternal(href);
      }}
    >
      {children}
    </a>
  );
});
