// The "Chat Clients" tab of the Badges overlay: every badge a third-party chat
// client hands its developers and supporters (FFZ, Chatterino, Homies,
// Moltorino, Chatsen, Chatty, DankChat), one framed section per client.
//
// Data is the Rust gallery (`get_all_third_party_badges`): one entry per
// DISTINCT badge with a holder count and whether the signed-in viewer holds
// it. BetterTTV rides the same command but has its own tab, so it is not in
// the provider list below.
//
// Layout follows the house glass rules: a `.glass-panel`
// frames each client, tiles are for-show `.glass-tile` surfaces (not buttons,
// the only action is the "open site" button in the header), colour lives on
// the mark, and the only accent is the owned ring. ~46 tiles across seven
// clients, so nothing collapses; search narrows within every section.

import { Check, ExternalLink } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
// Each client's own mark, bundled like the Chatterino and BetterTTV logos
// already were (sourced 2026-09-07 from each project's site or repo icon;
// 64 px, alpha, trimmed). Brand surfaces use the product's own mark, not a
// generic stand-in (Design_System.md).
import chatterinoLogo from '../assets/chatterino-logo.svg';
import homiesLogo from '../assets/homies-logo.png';
import moltorinoLogo from '../assets/moltorino-logo.svg';
import ffzLogo from '../assets/ffz-logo.png';
import dankchatLogo from '../assets/dankchat-logo.png';
import chatsenLogo from '../assets/chatsen-logo.png';
import chattyLogo from '../assets/chatty-logo.png';
import { Logger } from '../utils/logger';

export type ChatClientProvider =
  | 'ffz'
  | 'bttv'
  | 'chatterino'
  | 'homies'
  | 'moltorino'
  | 'chatsen'
  | 'chatty'
  | 'dankchat';

/** One distinct third-party chat-client badge (mirrors Rust ThirdPartyGalleryBadge). */
export interface ChatClientBadge {
  id: string;
  provider: ChatClientProvider;
  title: string;
  image_1x: string;
  image_2x: string;
  image_4x: string;
  user_count: number;
  owned: boolean;
  click_url: string | null;
}

interface ProviderMeta {
  key: Exclude<ChatClientProvider, 'bttv'>;
  label: string;
  /** One line under the name: who gets these and from where. */
  blurb: string;
  homepage: string;
  logo: string;
}

// Display order: the Chatterino family first (the client and its two forks read
// as one group), then the rest by how widely they are seen in chat.
const PROVIDERS: ProviderMeta[] = [
  {
    key: 'chatterino',
    label: 'Chatterino',
    blurb: 'Developers, contributors and supporters of the Chatterino desktop client.',
    homepage: 'https://chatterino.com/',
    logo: chatterinoLogo,
  },
  {
    key: 'homies',
    label: 'Homies',
    blurb: 'The Chatterino Homies fork: its developers, founders and supporters.',
    homepage: 'https://chatterinohomies.com/',
    logo: homiesLogo,
  },
  {
    key: 'moltorino',
    label: 'Moltorino',
    blurb: 'Supporter tiers from the Moltorino fork of Chatterino.',
    homepage: 'https://moltorino.com/',
    logo: moltorinoLogo,
  },
  {
    key: 'ffz',
    label: 'FrankerFaceZ',
    blurb: 'FrankerFaceZ developer, supporter and bot badges.',
    homepage: 'https://www.frankerfacez.com/badges',
    logo: ffzLogo,
  },
  {
    key: 'dankchat',
    label: 'DankChat',
    blurb: 'Badges from the DankChat Android client.',
    homepage: 'https://github.com/flex3r/DankChat',
    logo: dankchatLogo,
  },
  {
    key: 'chatsen',
    label: 'Chatsen',
    blurb: 'Patreon tiers from the Chatsen mobile client.',
    homepage: 'https://chatsen.app',
    logo: chatsenLogo,
  },
  {
    key: 'chatty',
    label: 'Chatty',
    blurb: 'Chatty supporter colours, plus the FrankerFaceZ badges it re-hosts.',
    homepage: 'https://chatty.github.io',
    logo: chattyLogo,
  },
];

interface ChatClientsGalleryProps {
  badges: ChatClientBadge[];
  /** Live search text from the overlay header; matched against badge titles. */
  query: string;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}

async function openExternal(url: string) {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(url);
  } catch (err) {
    Logger.error('Failed to open chat-client site:', err);
  }
}

function ProviderMark({ meta }: { meta: ProviderMeta }) {
  return (
    <div className="glass-tile flex h-9 w-9 flex-shrink-0 items-center justify-center overflow-hidden">
      <img src={meta.logo} alt="" className="h-6 w-6 object-contain" draggable={false} />
    </div>
  );
}

function BadgeTile({ badge }: { badge: ChatClientBadge }) {
  const holders =
    badge.user_count > 0
      ? `${badge.user_count.toLocaleString()} holder${badge.user_count === 1 ? '' : 's'}`
      : null;
  return (
    <Tooltip
      content={
        <div className="text-center">
          <div className="font-semibold">{badge.title}{badge.owned ? ' · Yours' : ''}</div>
          {holders && <div className="mt-0.5 text-[11px] text-textSecondary">{holders}</div>}
        </div>
      }
      side="top"
    >
      <div
        className={`glass-tile relative flex flex-col items-center gap-2 px-2 py-3 ${
          badge.owned ? 'ring-1 ring-inset ring-accent/60' : ''
        }`}
      >
        {badge.owned && (
          <div className="absolute right-1.5 top-1.5 flex h-4 w-4 items-center justify-center rounded-full bg-accent">
            <Check size={10} strokeWidth={3} className="text-background" />
          </div>
        )}
        {/* A dead upstream image (it happens: DankChat's "Top Supporter" 404s
            today) collapses to an empty box instead of the browser's broken
            glyph, so the title still identifies the badge. */}
        <img
          src={badge.image_4x || badge.image_2x || badge.image_1x}
          alt={badge.title}
          className="h-12 w-12 object-contain"
          loading="lazy"
          draggable={false}
          onError={(e) => { e.currentTarget.style.visibility = 'hidden'; }}
        />
        {/* Some titles are one unbreakable word ("maxVerstappenOnHisWay...");
            overflow-wrap lets the clamp actually clamp them. */}
        <span
          className={`line-clamp-2 w-full text-center text-[11px] leading-snug [overflow-wrap:anywhere] ${
            badge.owned ? 'font-medium text-textPrimary' : 'text-textSecondary'
          }`}
        >
          {badge.title}
        </span>
      </div>
    </Tooltip>
  );
}

function ProviderSection({ meta, badges }: { meta: ProviderMeta; badges: ChatClientBadge[] }) {
  const owned = badges.filter((b) => b.owned).length;
  return (
    <section className="glass-panel p-4">
      <header className="mb-3 flex items-center gap-3">
        <ProviderMark meta={meta} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <h3 className="text-sm font-semibold text-textPrimary">{meta.label}</h3>
            <span className="text-[11px] tabular-nums text-textMuted">
              {badges.length} badge{badges.length === 1 ? '' : 's'}
            </span>
          </div>
          <p className="truncate text-xs text-textSecondary">{meta.blurb}</p>
        </div>
        {owned > 0 && (
          <span className="glass-badge flex flex-shrink-0 items-center gap-1 px-2 py-1 text-[11px] tabular-nums text-accent">
            <Check size={12} />
            {owned} yours
          </span>
        )}
        <Tooltip content={`Open ${meta.label}`} side="top">
          <button
            onClick={() => openExternal(meta.homepage)}
            className="glass-button flex h-8 w-8 flex-shrink-0 items-center justify-center text-textSecondary hover:text-textPrimary"
            aria-label={`Open ${meta.label}`}
          >
            <ExternalLink size={14} />
          </button>
        </Tooltip>
      </header>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-2">
        {badges.map((badge) => (
          <BadgeTile key={badge.id} badge={badge} />
        ))}
      </div>
    </section>
  );
}

export function ChatClientsGallery({ badges, query, loading, error, onRetry }: ChatClientsGalleryProps) {
  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center">
          <div className="mx-auto mb-4 h-12 w-12 animate-spin rounded-full border-b-2 border-accent" />
          <p className="text-textSecondary">Loading chat-client badges...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center">
          <p className="mb-4 text-red-400">{error}</p>
          <button onClick={onRetry} className="glass-button px-4 py-2 text-accent">
            Retry
          </button>
        </div>
      </div>
    );
  }

  const q = query.trim().toLowerCase();
  const sections = PROVIDERS.map((meta) => {
    const all = badges.filter((b) => b.provider === meta.key);
    const shown = q ? all.filter((b) => b.title.toLowerCase().includes(q)) : all;
    return { meta, badges: shown };
  }).filter((s) => s.badges.length > 0);

  if (sections.length === 0) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-textSecondary">
          {q ? `No chat-client badges match "${query.trim()}"` : 'No chat-client badges found'}
        </p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      <p className="px-1 text-xs text-textMuted">
        Badges other chat clients award their developers and supporters. When a holder chats, the
        badge shows beside their name in StreamNook.
      </p>
      {sections.map(({ meta, badges: shown }) => (
        <ProviderSection key={meta.key} meta={meta} badges={shown} />
      ))}
    </div>
  );
}
