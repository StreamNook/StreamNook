// The combined-chat panel under the chat header: suggested channels for this
// streamer on other platforms, and linking one by hand.
//
// It never opens on its own. The header's + button is the way in, and shows
// when something was found; a question pushed into chat on every stream would
// be in the way of the chat itself.
//
// It borrows the chat header's material rather than bringing its own. The
// header directly above is an ultra-blurred panel over a 90% mix of the
// background with a subtle bottom border, and a strip in any other material
// sitting flush beneath it reads as two surfaces arguing instead of one stack
// of chrome.

import type { ReactNode } from 'react';
import { useEffect, useId, useState } from 'react';
import { motion } from 'framer-motion';
import { CircleNotch, MagnifyingGlass, Plus, X } from 'phosphor-react';
import { ProviderMark } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import {
  isYouTubeChannelId,
  isYouTubeLegacyPath,
  kickSlugHasTwoSpellings,
  linkPlatformOf,
  parseLinkInput,
} from '../../utils/parseChannelInput';
import {
  lookupError,
  resolveKickSlug,
  resolveYouTubeIdentifier,
  youTubeChannelTitle,
} from '../../services/channelLookup';
import { searchPlatforms } from '../../services/platformSearch';
import type { BlendCompanion, LinkSuggestion } from '../../hooks/useBlendCompanions';

/** The platforms whose chat can be combined, in the order the picker lists them. */
const COMBINABLE: ProviderId[] = ['kick', 'youtube', 'twitch'];

const BAR_BG = { backgroundColor: 'color-mix(in srgb, var(--color-background) 90%, transparent)' };

/** What the channel typed into the box turned out to be. */
type Preview =
  | { state: 'idle' }
  | { state: 'looking' }
  | { state: 'found'; channel: string; name: string; avatar?: string; live: boolean | null; title: string }
  | { state: 'missing' }
  /** Not looked up: a YouTube @handle has no cheap offline lookup, or the
   *  platform could not be reached. Linking still works. */
  | { state: 'unchecked' };

const IDLE: Preview = { state: 'idle' };
const LOOKING: Preview = { state: 'looking' };

/** Ask the platform, through Rust's search, who a parsed channel is. */
async function lookUp(provider: ProviderId, channel: string): Promise<Preview> {
  if (provider === 'youtube') {
    if (!isYouTubeChannelId(channel)) return { state: 'unchecked' };
    const name = await youTubeChannelTitle(channel);
    return name ? { state: 'found', channel, name, live: null, title: '' } : { state: 'unchecked' };
  }
  let query = channel;
  if (provider === 'kick' && kickSlugHasTwoSpellings(channel)) {
    try {
      query = await resolveKickSlug(channel);
    } catch {
      return { state: 'missing' };
    }
  }
  return new Promise<Preview>((resolve) => {
    let settled = false;
    const settle = (p: Preview) => {
      if (!settled) {
        settled = true;
        resolve(p);
      }
    };
    searchPlatforms(query, [provider], (batch) => {
      if (batch.error) return settle({ state: 'unchecked' });
      const row = batch.streams.find((r) => r.user_login?.toLowerCase() === query.toLowerCase());
      settle(
        row
          ? {
              state: 'found',
              channel: row.user_login,
              name: row.user_name || row.user_login,
              avatar: row.profile_image_url || undefined,
              live: !!row.is_live,
              title: row.title ?? '',
            }
          : { state: 'missing' },
      );
    }).then(
      () => settle({ state: 'unchecked' }),
      () => settle({ state: 'unchecked' }),
    );
  });
}

/** Which platform the other channel is on. The pill slides between choices. */
function PlatformPicker({
  options,
  value,
  onChange,
}: {
  options: ProviderId[];
  value: ProviderId;
  onChange: (p: ProviderId) => void;
}) {
  const pillId = useId();
  return (
    <div className="blend-picker" role="radiogroup" aria-label="Platform">
      {options.map((p) => {
        const active = p === value;
        return (
          <button
            key={p}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(p)}
            className={`blend-picker-option ${active ? 'is-active' : ''}`}
          >
            {active && (
              <motion.span
                layoutId={`blend-picker-${pillId}`}
                className="blend-picker-pill"
                transition={{ type: 'spring', stiffness: 480, damping: 32 }}
              />
            )}
            <ProviderMark provider={p} size={11} className={active ? '' : 'opacity-60 grayscale'} />
            <span className="relative">{PROVIDERS[p].label}</span>
          </button>
        );
      })}
    </div>
  );
}

const PLACEHOLDER: Partial<Record<ProviderId, string>> = {
  kick: 'Kick channel name or link',
  youtube: '@handle or channel link',
  twitch: 'Twitch username or link',
};

/** Linking a channel: pick the platform, type or paste, see who it is, link. */
function LinkEditor({
  homeProvider,
  linked,
  onAdd,
  onRemove,
  onClose,
}: {
  homeProvider: ProviderId;
  linked: BlendCompanion[];
  onAdd: (provider: ProviderId, channel: string, extra?: { display_name?: string; avatar?: string }) => void;
  onRemove: (c: BlendCompanion) => void;
  onClose: () => void;
}) {
  const options = COMBINABLE.filter((p) => p !== homeProvider);
  const [picked, setPicked] = useState<ProviderId>(options[0]);
  const [draft, setDraft] = useState('');
  // Set while a submit is looked up (a legacy YouTube link); a failure shows inline.
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // A pasted link names its own platform, so it moves the picker with it. A link
  // to somewhere combined chat cannot reach, or a URL that names no platform, is
  // said plainly rather than read as a very strange Kick name.
  const trimmed = draft.trim();
  const linkedTo = linkPlatformOf(trimmed);
  const unsupported = linkedTo !== null && !options.includes(linkedTo);
  const strayUrl = linkedTo === null && /[/:]/.test(trimmed);
  const parsed = unsupported || strayUrl ? null : parseLinkInput(trimmed, picked);
  const already =
    !!parsed &&
    linked.some((c) => c.provider === parsed.provider && c.channel.toLowerCase() === parsed.channel.toLowerCase());

  // The lookup result is stamped with what it was for, so a stale answer never
  // shows against newer text and nothing is set synchronously in the effect.
  const lookupKey = parsed && !already ? `${parsed.provider}:${parsed.channel}` : '';
  const [result, setResult] = useState<{ key: string; preview: Preview } | null>(null);
  const preview: Preview = !lookupKey ? IDLE : result?.key === lookupKey ? result.preview : LOOKING;
  const lookupProvider = parsed?.provider;
  const lookupChannel = parsed?.channel;
  useEffect(() => {
    if (!lookupKey || !lookupProvider || !lookupChannel) return;
    let cancelled = false;
    // Typing pauses before asking, so a name is looked up once, not per letter.
    const timer = window.setTimeout(() => {
      void lookUp(lookupProvider, lookupChannel).then((p) => {
        if (!cancelled) setResult({ key: lookupKey, preview: p });
      });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [lookupKey, lookupProvider, lookupChannel]);

  const canLink = !!parsed && !already && !busy && preview.state !== 'missing' && preview.state !== 'looking';

  const submit = async () => {
    if (!parsed || !canLink) return;
    if (preview.state === 'found') {
      onAdd(parsed.provider, preview.channel, { display_name: preview.name, avatar: preview.avatar });
      setDraft('');
      return;
    }
    let channel = parsed.channel;
    // A legacy YouTube /c/ or /user/ link names no channel until YouTube says which.
    if (parsed.provider === 'youtube' && isYouTubeLegacyPath(channel)) {
      setBusy(true);
      try {
        channel = await resolveYouTubeIdentifier(channel);
      } catch (err) {
        setError(lookupError(err, 'YouTube'));
        return;
      } finally {
        setBusy(false);
      }
    }
    onAdd(parsed.provider, channel);
    setDraft('');
  };

  const shownProvider = parsed?.provider ?? picked;
  const platformName = PROVIDERS[shownProvider].label;

  let status: ReactNode;
  if (error) {
    status = <span className="blend-status-text blend-status-text--error">{error}</span>;
  } else if (unsupported && linkedTo === homeProvider) {
    status = (
      <span className="blend-status-text">
        This chat is already {PROVIDERS[linkedTo].label}. Link the streamer's channel on another platform.
      </span>
    );
  } else if (unsupported && linkedTo) {
    status = (
      <span className="blend-status-text">
        {PROVIDERS[linkedTo].label} chat can't be combined here. Try {options.map((p) => PROVIDERS[p].label).join(' or ')}.
      </span>
    );
  } else if (strayUrl) {
    status = (
      <span className="blend-status-text">
        That link doesn't name a {options.map((p) => PROVIDERS[p].label).join(', ')} channel.
      </span>
    );
  } else if (already) {
    status = <span className="blend-status-text">Already combined with this chat.</span>;
  } else if (preview.state === 'looking') {
    status = (
      <span className="blend-status-text">
        <CircleNotch size={12} className="animate-spin" />
        Looking up {parsed?.channel} on {platformName}
      </span>
    );
  } else if (preview.state === 'found') {
    status = (
      <>
        {preview.avatar ? (
          <img
            src={preview.avatar}
            alt=""
            className="h-6 w-6 shrink-0 rounded-full object-cover"
            onError={(e) => {
              e.currentTarget.style.display = 'none';
            }}
          />
        ) : (
          <ProviderMark provider={shownProvider} size={14} />
        )}
        <span className="flex min-w-0 flex-col">
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-[12px] font-semibold text-textPrimary">{preview.name}</span>
            <ProviderMark provider={shownProvider} size={10} className="shrink-0" />
          </span>
          <span className="flex min-w-0 items-center gap-1 text-[10.5px] text-textMuted">
            {preview.live === true && <span className="blend-live-dot" aria-hidden />}
            <span className="truncate">
              {preview.live === true
                ? preview.title
                  ? `Live: ${preview.title}`
                  : 'Live now'
                : preview.live === false
                  ? 'Offline'
                  : `${platformName} channel`}
            </span>
          </span>
        </span>
      </>
    );
  } else if (preview.state === 'missing') {
    status = (
      <span className="blend-status-text blend-status-text--error">
        No {platformName} channel called {parsed?.channel}.
      </span>
    );
  } else if (preview.state === 'unchecked' && parsed) {
    status = (
      <span className="blend-status-text">
        <ProviderMark provider={shownProvider} size={12} />
        <span className="truncate">
          <span className="text-textPrimary">{parsed.channel}</span> on {platformName}
        </span>
      </span>
    );
  } else {
    status = (
      <span className="blend-status-text">
        Its messages join this chat, each marked with its platform.
      </span>
    );
  }

  return (
    <div className="blend-editor border-b border-borderSubtle backdrop-blur-ultra" style={BAR_BG}>
      <div className="flex items-center gap-2">
        <span className="blend-editor-label">Combine chat</span>
        <button type="button" onClick={onClose} className="blend-done">
          Done
        </button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {options.length > 1 && (
          <PlatformPicker
            options={options}
            value={parsed?.provider && options.includes(parsed.provider) ? parsed.provider : picked}
            onChange={(p) => {
              setPicked(p);
              setError(null);
            }}
          />
        )}
        <label className="blend-field">
          <MagnifyingGlass size={13} weight="bold" className="shrink-0 text-textMuted" />
          <input
            autoFocus
            value={draft}
            onChange={(e) => {
              const v = e.target.value;
              setDraft(v);
              setError(null);
              const lp = linkPlatformOf(v.trim());
              if (lp && options.includes(lp)) setPicked(lp);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void submit();
              if (e.key === 'Escape') onClose();
            }}
            placeholder={PLACEHOLDER[picked] ?? 'Channel name or link'}
            aria-label={`${PROVIDERS[picked].label} channel`}
            spellCheck={false}
          />
          <button type="button" onClick={() => void submit()} disabled={!canLink} className="blend-link-btn">
            Link
          </button>
        </label>
      </div>

      <div className="blend-status" aria-live="polite">
        {status}
      </div>

      {/* Removing a link is rare, so it gets no permanent control; it lives
          here, in the one place someone is already managing them. */}
      {linked.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="blend-editor-label mr-0.5">Linked</span>
          {linked.map((c) => (
            <span key={`${c.provider}:${c.channel}`} className="blend-chip">
              <ProviderMark provider={c.provider} size={11} />
              <span className="max-w-[9rem] truncate">{c.channelName}</span>
              <Tooltip content={`Unlink ${c.channelName} on ${PROVIDERS[c.provider].label}`} side="bottom">
                <button
                  type="button"
                  onClick={() => onRemove(c)}
                  className="blend-chip-remove"
                  aria-label={`Unlink ${c.channelName} on ${PROVIDERS[c.provider].label}`}
                >
                  <X size={9} weight="bold" />
                </button>
              </Tooltip>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** "Is this the same streamer?" for one platform.
 *
 *  Enough to tell a same-named stranger apart before linking them: the face,
 *  the name as it is written there, and what they are streaming. A yes/no on a
 *  bare slug would be a guess. Two lines so a narrow chat truncates the title,
 *  never the name or the buttons. */
function SuggestionRow({
  suggestion,
  onAccept,
  onRefuse,
}: {
  suggestion: LinkSuggestion;
  onAccept: () => void;
  onRefuse: () => void;
}) {
  const { candidate } = suggestion;
  const platform = PROVIDERS[candidate.provider].label;
  const name = candidate.display_name || candidate.channel;
  return (
    <div className="blend-suggest">
      <span className="blend-suggest-face">
        {candidate.avatar ? (
          <img
            src={candidate.avatar}
            alt=""
            onError={(e) => {
              e.currentTarget.style.visibility = 'hidden';
            }}
          />
        ) : null}
        <span className="blend-suggest-badge">
          <ProviderMark provider={candidate.provider} size={10} />
        </span>
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-[12px] font-semibold leading-4 text-textPrimary">{name}</span>
        <span className="flex min-w-0 items-center gap-1 text-[10.5px] leading-4 text-textMuted">
          {suggestion.is_live && <span className="blend-live-dot" aria-hidden />}
          <span className="truncate">
            {suggestion.is_live
              ? `Live on ${platform}${suggestion.title ? `: ${suggestion.title}` : ''}`
              : `On ${platform}, offline right now`}
          </span>
        </span>
      </span>
      <Tooltip content={`Same streamer: combine their ${platform} chat with this one`} side="bottom">
        <button type="button" onClick={onAccept} className="blend-action blend-action--primary">
          Combine
        </button>
      </Tooltip>
      <Tooltip content={`Wrong channel. This ${platform} channel won't be suggested again.`} side="bottom">
        <button type="button" onClick={onRefuse} className="blend-action">
          Not them
        </button>
      </Tooltip>
    </div>
  );
}

/** What the combined-chat panel under the header is showing. */
export type BlendView = 'closed' | 'suggestions' | 'editor';

export function BlendBar({
  view,
  onViewChange,
  homeProvider,
  linked,
  onAdd,
  onRemove,
  suggestions = [],
  onAcceptSuggestion,
  onRefuseSuggestion,
}: {
  /** Opened only from the chat header's button; nothing here opens itself. */
  view: BlendView;
  onViewChange: (view: BlendView) => void;
  /** The platform of the channel being watched; it is never offered as a link. */
  homeProvider: ProviderId;
  /** Every platform linked to this streamer. */
  linked: BlendCompanion[];
  onAdd: (provider: ProviderId, channel: string, extra?: { display_name?: string; avatar?: string }) => void;
  onRemove: (c: BlendCompanion) => void;
  /** Channels that might be this streamer elsewhere, at most one per platform.
   *  Never linked on their own. */
  suggestions?: LinkSuggestion[];
  onAcceptSuggestion: (s: LinkSuggestion) => void;
  onRefuseSuggestion: (s: LinkSuggestion) => void;
}) {
  if (view === 'editor') {
    return (
      <LinkEditor
        homeProvider={homeProvider}
        linked={linked}
        onAdd={onAdd}
        onRemove={onRemove}
        onClose={() => onViewChange('closed')}
      />
    );
  }

  if (view === 'suggestions' && suggestions.length > 0) {
    return (
      <div className="border-b border-borderSubtle backdrop-blur-ultra" style={BAR_BG}>
        {/* Hiding and refusing are different answers, so they are different
            controls: Hide only folds the panel away (the suggestions wait
            behind the header button), Not them is "wrong channel" and is
            remembered for that one channel. */}
        <div className="flex items-center gap-2 px-3 pt-2">
          <span className="blend-editor-label">Same streamer elsewhere?</span>
          <Tooltip content="Put these away. They stay behind the + in the header." side="bottom">
            <button type="button" onClick={() => onViewChange('closed')} className="blend-done">
              Hide
            </button>
          </Tooltip>
        </div>
        <div className="hairline-y">
          {suggestions.map((s) => (
            <SuggestionRow
              key={s.candidate.provider}
              suggestion={s}
              onAccept={() => onAcceptSuggestion(s)}
              onRefuse={() => onRefuseSuggestion(s)}
            />
          ))}
        </div>
        <div className="px-3 pb-2">
          <button type="button" onClick={() => onViewChange('editor')} className="blend-more">
            <Plus size={9} weight="bold" />
            Link a different channel
          </button>
        </div>
      </div>
    );
  }

  return null;
}

export default BlendBar;
