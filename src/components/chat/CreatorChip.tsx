// A person who made an emote, as a button: their picture, their name, and
// where the click goes. A Twitch account opens StreamNook's own profile view
// (in main; a popout raises it); 7TV, BetterTTV and FFZ creators open their
// page on that site. Rust built and checked every link this receives.
import { useState } from 'react';
import { ExternalLink, UserRound } from 'lucide-react';

import type { EmoteCreator } from '../../services/emoteCredit';
import { openExternal } from '../../utils/openExternal';

/** Two letters for a picture that is missing or failed to load. */
function initials(name: string): string {
  const parts = name.replace(/[^\p{L}\p{N} _]/gu, '').split(/[\s_]+/).filter(Boolean);
  const text = parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2);
  return text.toUpperCase();
}

function CreatorAvatar({ name, url, px }: { name: string; url?: string; px: number }) {
  const [failed, setFailed] = useState(false);
  const box = { width: px, height: px };
  if (url && !failed) {
    return (
      <img
        src={url}
        alt=""
        style={box}
        loading="lazy"
        referrerPolicy="no-referrer"
        draggable={false}
        onError={() => setFailed(true)}
        className="shrink-0 rounded-full object-cover bg-glass"
      />
    );
  }
  return (
    <span
      style={box}
      aria-hidden
      className="flex shrink-0 items-center justify-center rounded-full bg-accent/15 text-[10px] font-semibold text-accent"
    >
      {initials(name)}
    </span>
  );
}

/** Where a creator's click goes, as the line under their name. */
function destination(creator: EmoteCreator): string {
  return creator.link.kind === 'twitch' ? 'Twitch profile' : `${creator.link.site} profile`;
}

function openCreator(creator: EmoteCreator, onLeave?: () => void): void {
  const link = creator.link;
  if (link.kind === 'web') {
    void openExternal(link.url);
    return;
  }
  if (link.id) {
    onLeave?.();
    void import('../../utils/openBadgesInMain').then((m) =>
      m.openTwitchProfileInMain({ id: link.id!, login: link.login, displayName: creator.name, avatarUrl: creator.avatarUrl }),
    );
    return;
  }
  // No account id to open in the app: Twitch's own page still finds them.
  void openExternal(`https://www.twitch.tv/${link.login}`);
}

/**
 * Full width (the emote card): picture, name, and a quiet line saying where it
 * goes. `compact` is the inline pill for a header line ("by name").
 */
export function CreatorChip({
  creator,
  compact = false,
  onLeave,
}: {
  creator: EmoteCreator;
  compact?: boolean;
  /** Runs before a click leaves for the in-app profile (close the card). */
  onLeave?: () => void;
}) {
  const external = creator.link.kind === 'web';
  const Icon = external ? ExternalLink : UserRound;
  const label = `${creator.name}, ${destination(creator)}`;
  if (compact) {
    return (
      <button
        type="button"
        onClick={() => openCreator(creator, onLeave)}
        aria-label={label}
        className="group inline-flex max-w-full items-center gap-1.5 rounded-full py-0.5 pl-0.5 pr-2 text-xs text-textSecondary transition-colors hover:bg-glass hover:text-textPrimary focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70"
      >
        <CreatorAvatar name={creator.name} url={creator.avatarUrl} px={16} />
        <span className="truncate">by {creator.name}</span>
        <Icon size={11} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-70 group-focus-visible:opacity-70" />
      </button>
    );
  }
  return (
    <button
      type="button"
      onClick={() => openCreator(creator, onLeave)}
      aria-label={label}
      className="group flex w-full items-center gap-2.5 rounded-lg border border-borderSubtle bg-glass/40 px-2.5 py-2 text-left transition-colors hover:border-white/20 hover:bg-glass focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70"
    >
      <CreatorAvatar name={creator.name} url={creator.avatarUrl} px={28} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-[13px] font-medium leading-tight text-textPrimary">{creator.name}</span>
        <span className="truncate text-[11px] leading-tight text-textSecondary">{destination(creator)}</span>
      </span>
      <Icon size={14} className="shrink-0 text-textMuted transition-colors group-hover:text-textPrimary" />
    </button>
  );
}
