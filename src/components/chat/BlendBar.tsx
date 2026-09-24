// The control for combined chat: a slim bar under the chat header that turns
// the streamer's other platforms on and off in the feed.
//
// It only exists when there is something to combine — a channel with no linked
// platforms never sees it — so ordinary single-platform chat gains no chrome.
//
// It deliberately borrows the chat header's material rather than bringing its
// own. The header directly above is an ultra-blurred panel over a 90% mix of
// the background with a subtle bottom border, and a strip in any other material
// sitting flush beneath it reads as two surfaces arguing instead of one stack of
// chrome. Same reason the controls here are text and small marks: the header's
// vocabulary is a status dot, an 13px logo and a quiet label, so a row of
// filled brand-coloured pills under it would belong to a different app.

import type { ReactNode } from 'react';
import { useState } from 'react';
import { ProviderMark } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import { isYouTubeLegacyPath, kickSlugHasTwoSpellings, parseLinkInput } from '../../utils/parseChannelInput';
import { lookupError, resolveKickSlug, resolveYouTubeIdentifier } from '../../services/channelLookup';
import type { BlendCompanion, LinkSuggestion } from '../../hooks/useBlendCompanions';

/** The bar itself, in the chat header's material so the two read as one stack. */
function Row({ children }: { children: ReactNode }) {
  return (
    <div
      className="flex items-center gap-2.5 border-b border-borderSubtle px-3 py-1 backdrop-blur-ultra"
      style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 90%, transparent)' }}
    >
      {children}
    </div>
  );
}

/** A text control at the weight the header's own controls use. */
function Action({
  children,
  onClick,
  primary = false,
  disabled = false,
  label,
  className = '',
}: {
  children: ReactNode;
  onClick: () => void;
  primary?: boolean;
  disabled?: boolean;
  label?: string;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      className={`shrink-0 whitespace-nowrap text-[11px] transition-colors disabled:text-textMuted disabled:opacity-50 ${
        primary ? 'text-accent hover:brightness-110' : 'text-textMuted hover:text-textSecondary'
      } ${className}`}
    >
      {children}
    </button>
  );
}

export function BlendBar({
  linked,
  onAdd,
  onRemove,
  invite = false,
  onInviteClose,
  suggestion,
  onAcceptSuggestion,
  onRefuseSuggestion,
}: {
  /** Every platform linked to this streamer. */
  linked: BlendCompanion[];
  onAdd: (provider: ProviderId, channel: string) => void;
  onRemove: (c: BlendCompanion) => void;
  /** Offer to manage links. The host shows this only when it is worth a line:
   *  a channel with nothing linked yet, or a deliberate request to edit them. */
  invite?: boolean;
  /** Told when the viewer is done editing, so the host can put the line away. */
  onInviteClose?: () => void;
  /** A channel that might be this streamer elsewhere. Never linked on its own. */
  suggestion?: LinkSuggestion | null;
  onAcceptSuggestion: (s: LinkSuggestion) => void;
  onRefuseSuggestion: (s: LinkSuggestion) => void;
}) {
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState('');
  // Set while an input is looked up (a legacy YouTube link, or a Kick name with
  // two spellings); a failed lookup shows inline.
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const parsed = parseLinkInput(draft);

  const closeEditor = () => {
    setDraft('');
    setError(null);
    setAdding(false);
    onInviteClose?.();
  };

  const submit = async () => {
    if (!parsed || busy) return;
    let channel = parsed.channel;
    // Two inputs name no channel until the platform says which: a legacy
    // YouTube /c/ or /user/ link, and a Kick name that can be spelled two ways.
    const youtube = parsed.provider === 'youtube' && isYouTubeLegacyPath(channel);
    const kick = parsed.provider === 'kick' && kickSlugHasTwoSpellings(channel);
    if (youtube || kick) {
      setBusy(true);
      try {
        channel = youtube ? await resolveYouTubeIdentifier(channel) : await resolveKickSlug(channel);
      } catch (err) {
        setError(lookupError(err, PROVIDERS[parsed.provider].label));
        return;
      } finally {
        setBusy(false);
      }
    }
    onAdd(parsed.provider, channel);
    closeEditor();
  };

  if (suggestion) {
    const meta = PROVIDERS[suggestion.candidate.provider];
    return (
      <Row>
        {/* Enough to tell a same-named stranger apart before linking them: the
            face, the name as it is written there, and what they are streaming.
            A yes/no on a bare slug would be a guess. */}
        {suggestion.candidate.avatar && (
          <img
            src={suggestion.candidate.avatar}
            alt=""
            className="h-4 w-4 shrink-0 rounded-full object-cover"
            onError={(e) => {
              e.currentTarget.style.display = 'none';
            }}
          />
        )}
        <ProviderMark provider={suggestion.candidate.provider} size={11} />
        <span className="min-w-0 flex-1 truncate text-[11px] text-textSecondary">
          <span className="text-textPrimary">
            {suggestion.candidate.display_name || suggestion.candidate.channel}
          </span>
          {` on ${meta.label}`}
          {suggestion.is_live && suggestion.title ? ` — ${suggestion.title}` : ''}
          {suggestion.is_live ? '' : ' (offline)'}
        </span>
        <Action onClick={() => onAcceptSuggestion(suggestion)} primary>
          Same streamer
        </Action>
        <Action onClick={() => onRefuseSuggestion(suggestion)}>Not them</Action>
      </Row>
    );
  }

  // The host asking to manage links means the viewer already pressed something
  // that says so. Landing them on a line that offers to open the form would be
  // a second click for the same intent, so go straight to it; the empty-state
  // offer below is different, because there it has to explain itself first.
  if (adding || (invite && linked.length > 0)) {
    return (
      <Row>
        <input
          autoFocus
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void submit();
            if (e.key === 'Escape') closeEditor();
          }}
          placeholder="kick.com/name, @handle or a link"
          className="min-w-0 flex-1 rounded border border-borderSubtle bg-black/20 px-2 py-0.5 text-[11px] text-textPrimary outline-none placeholder:text-textMuted focus:border-textMuted"
        />
        {/* Which platform this will be linked as, decided BEFORE it is linked:
            a bare name is read as Kick and a link names its own. */}
        {error ? (
          <span className="max-w-[14rem] shrink truncate text-[11px] text-error" title={error}>
            {error}
          </span>
        ) : (
          parsed && <ProviderMark provider={parsed.provider} size={11} />
        )}
        {/* Removing a link is rare, so it does not get a permanent control; it
            lives here, in the one place someone is already managing them. */}
        {linked.map((c) => (
          <Tooltip
            key={`${c.provider}:${c.channel}`}
            content={`Unlink ${c.channelName} on ${PROVIDERS[c.provider].label}`}
            side="bottom"
          >
            <button
              type="button"
              onClick={() => onRemove(c)}
              className="inline-flex shrink-0 items-center gap-1 text-[11px] text-textMuted transition-colors hover:text-textPrimary"
            >
              <ProviderMark provider={c.provider} size={11} className="opacity-60" />
              &times;
            </button>
          </Tooltip>
        ))}
        <Action onClick={() => void submit()} primary disabled={!parsed || busy}>
          Link
        </Action>
        <Action onClick={closeEditor} label="Done">
          &times;
        </Action>
      </Row>
    );
  }

  // Nothing to say. The steady state — which platforms are in the feed, and
  // whether it is combined at all — is in the chat header now, where the label
  // already had to change anyway. A permanent second line to repeat it would be
  // chrome for its own sake.
  if (!invite) return null;

  return (
    <Row>
      <Action onClick={() => setAdding(true)}>Combine chat from another platform</Action>
    </Row>
  );
}

export default BlendBar;
