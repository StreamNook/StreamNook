// The "also on your other devices" links: one pill per platform this build is
// not, each to the downloads page. Shared by the desktop About popup and the
// phone's Settings > Support.
import { ExternalAnchor } from './ui/ExternalAnchor';
import { PlatformLogo } from './ui/PlatformLogo';
import { DOWNLOADS_URL, type PlatformEntry } from '../hooks/useClientPlatform';

interface OtherPlatformLinksProps {
  platforms: PlatformEntry[];
  /** Phone-sized pills: a finger needs more than the desktop's 24px. */
  large?: boolean;
  className?: string;
}

export function OtherPlatformLinks({ platforms, large = false, className = '' }: OtherPlatformLinksProps) {
  return (
    // Two equal columns, so four pills always split 2 + 2 rather than leaving
    // one alone on a row in the narrow popup or on a phone.
    <div className={`grid w-fit grid-cols-2 gap-2 ${className}`}>
      {platforms.map((p) => (
        <ExternalAnchor
          key={p.key}
          href={DOWNLOADS_URL}
          className={`inline-flex items-center rounded-full bg-glass font-medium text-textSecondary transition-colors hover:bg-glass-hover hover:text-textPrimary ${
            // Phone pills sit under a left-aligned title, so their logos line
            // up on the left; the popup's are centered, so theirs are too.
            large ? 'justify-start gap-2 px-3.5 py-2 text-[13px]' : 'justify-center gap-1.5 px-3 py-1 text-xs'
          }`}
        >
          <PlatformLogo platform={p.key} size={large ? 15 : 13} />
          {p.name}
        </ExternalAnchor>
      ))}
    </div>
  );
}
