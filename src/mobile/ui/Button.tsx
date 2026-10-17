// The phone's worded buttons, one system: every one is a capsule, so swapping
// one for another never moves what is around it. Page buttons are 50px with a
// 16/600 label (the iPhone's SNButtonStyle); row buttons, the small pills at the
// end of a row (Sign out, Change, Connect), are 34px with a 14/600 label.
//
// - primary: the accent fill. One per screen at most: the way on.
// - secondary: glass. Page actions beside a primary, row actions, or the only
//   action where it should not shout.
// - brand: a platform's own color, for its sign-in only ("Sign in with Kick").
//
// A status is not a button: "Connected" or "Notifications are on" is a mark and
// words, never a disabled capsule.
import React, { forwardRef } from 'react';
import { PROVIDERS, type ProviderId } from '../../types/providers';

export type ButtonKind = 'primary' | 'secondary' | 'brand';
export type ButtonSize = 'page' | 'row';

/** Label color on each brand's fill, picked for contrast against it. */
const BRAND_LABEL: Partial<Record<ProviderId, string>> = { kick: '#000000' };

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  kind?: ButtonKind;
  size?: ButtonSize;
  /** The platform whose color a `brand` button wears. */
  brand?: ProviderId;
  /** A leading glyph (a platform mark on a brand button). */
  icon?: React.ReactNode;
}

const SIZE: Record<ButtonSize, string> = {
  page: 'h-[50px] px-6 text-[16px] gap-2',
  row: 'h-[34px] px-4 text-[14px] gap-1.5',
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ kind = 'secondary', size = 'page', brand, icon, className = '', style, children, type = 'button', ...rest }, ref) => {
    const look =
      kind === 'primary'
        ? 'bg-accent text-background'
        : kind === 'brand'
          ? ''
          : 'sn-button-secondary text-textPrimary';
    const brandStyle =
      kind === 'brand' && brand
        ? { backgroundColor: PROVIDERS[brand].color, color: BRAND_LABEL[brand] ?? '#ffffff' }
        : undefined;
    return (
      <button
        ref={ref}
        type={type}
        className={`inline-flex shrink-0 items-center justify-center rounded-full font-semibold leading-none transition-opacity active:opacity-75 disabled:opacity-50 ${SIZE[size]} ${look} ${className}`}
        style={{ ...brandStyle, ...style }}
        {...rest}
      >
        {icon && <span className="flex shrink-0">{icon}</span>}
        {children}
      </button>
    );
  },
);
Button.displayName = 'Button';
