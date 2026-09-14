// The one "connect this platform" button, wherever the app asks for an account.
//
// It used to be a slab of the platform's own brand colour with white text: a
// Kick button was a rectangle of #53fc18, a Twitch one a rectangle of #9146ff.
// That is how the platforms style their own sites, and it is wrong inside a
// dark app with a user-chosen accent, because a fully saturated brand fill is
// louder than anything else on the screen and belongs to a design system that
// is not ours.
//
// Here the brand colour is a light source rather than a fill: a low wash and a
// hairline of it, with the real mark carrying the identity at full strength.
// Same rule the title-bar platform switcher follows.

import { ProviderMark } from './ProviderLogo';
import { PROVIDERS, providerLabel, type ProviderId } from '../types/providers';

export function PlatformLoginButton({
  provider,
  onClick,
  busy = false,
  busyLabel = 'Connecting…',
  label,
  size = 'md',
}: {
  provider: ProviderId;
  onClick: () => void;
  busy?: boolean;
  busyLabel?: string;
  /** Defaults to "Login with <Platform>". */
  label?: string;
  /** `sm` is the inline pill used in a chat composer; `md` is the CTA in an
   *  empty state. */
  size?: 'sm' | 'md';
}) {
  const tint = PROVIDERS[provider].color;
  const wash = (pct: number) => `color-mix(in srgb, ${tint} ${pct}%, transparent)`;
  const small = size === 'sm';
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className={`group inline-flex items-center justify-center rounded-lg font-medium text-textPrimary transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-60 ${
        small ? 'gap-1.5 px-2.5 py-1 text-xs' : 'gap-2 px-4 py-2 text-sm'
      }`}
      style={{
        backgroundColor: wash(13),
        // An inset ring rather than a border, so the button's box does not grow
        // by 2px against everything laid out beside it.
        boxShadow: `inset 0 0 0 1px ${wash(30)}`,
      }}
      onMouseEnter={(e) => {
        if (busy) return;
        e.currentTarget.style.backgroundColor = wash(22);
        e.currentTarget.style.boxShadow = `inset 0 0 0 1px ${wash(48)}`;
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.backgroundColor = wash(13);
        e.currentTarget.style.boxShadow = `inset 0 0 0 1px ${wash(30)}`;
      }}
    >
      <ProviderMark provider={provider} size={small ? 13 : 16} />
      <span>{busy ? busyLabel : (label ?? `Login with ${providerLabel(provider)}`)}</span>
    </button>
  );
}

export default PlatformLoginButton;
