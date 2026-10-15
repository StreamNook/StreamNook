// Your own identity, at the top of the You tab: avatar, 7TV-painted display
// name, and the badges you actually carry.
//
// Rust builds it (`own_identity_header`, services/profile_view.rs): Twitch's
// displayed badges minus the channel-only sets, the selected 7TV badge, your
// StreamNook Identity loadout (or your raw chat-client badges when you have
// none), deduped and capped at eight, StreamNook last. It saves the answer for
// the next cold launch, so `own_identity_header_cached` paints the first frame
// and the fresh read follows. The iPhone draws the same answer.
import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { computePaintStyle } from '../../services/seventvService';
import { subscribeResolvedIdentity } from '../../services/identityService';
import type { OwnIdentityView } from '../../types/profileView';
import { FallbackImage } from '../../components/FallbackImage';
import { StreamNookBadge } from '../../components/StreamNookBadge';
import { Logger } from '../../utils/logger';

// The avatar stays with YouScreen: it is the one piece that needs no cosmetics
// lookup, and keeping it there lets the layout own its own alignment.
interface Props {
  userId: string;
  displayName: string;
  login?: string;
}

/**
 * Type size for the name.
 *
 * Twitch caps display names at 25 characters, so this is a bounded problem: the
 * longest possible name simply needs a smaller size to keep the header to two
 * lines. Stepping the size beats truncating, because the name is the one thing
 * on this screen the viewer came to see, and an ellipsis in your own name reads
 * like a bug.
 */
function nameSizeClass(name: string): string {
  const n = name.length;
  if (n > 20) return 'text-[14.5px]';
  if (n > 16) return 'text-[16px]';
  if (n > 12) return 'text-[17px]';
  return 'text-[18.5px]';
}

export const OwnIdentityHeader: React.FC<Props> = ({ userId, displayName, login }) => {
  const [identity, setIdentity] = useState<OwnIdentityView | null>(null);

  useEffect(() => {
    let cancelled = false;
    const selfLogin = login || displayName;
    const refresh = () =>
      invoke<OwnIdentityView>('own_identity_header', { userId, login: selfLogin, displayName })
        .then((fresh) => {
          if (!cancelled) setIdentity(fresh);
        })
        .catch((err) => Logger.debug('[OwnIdentity] header unavailable:', err));
    void invoke<OwnIdentityView | null>('own_identity_header_cached', { userId })
      .then((cached) => {
        if (!cancelled && cached) setIdentity((now) => now ?? cached);
      })
      .catch(() => {})
      .finally(() => void refresh());
    // A change elsewhere (a badge equipped, the loadout edited, a grant landing)
    // re-reads the header without a remount.
    const unsubscribe = subscribeResolvedIdentity((changedId) => {
      if (changedId === userId) void refresh();
    });
    const unlisten = listen('sn-registry-changed', () => void refresh());
    return () => {
      cancelled = true;
      unsubscribe();
      void unlisten.then((fn) => fn());
    };
  }, [userId, login, displayName]);

  const paint = identity?.paint_def;
  // No paint is the common case and must stay legible: fall back to the primary
  // text colour rather than a chat username colour, because this is a heading
  // on a settings screen, not a chat line.
  const nameStyle = paint
    ? computePaintStyle(paint, 'var(--color-text-primary)', 'all')
    : undefined;
  const badges = identity?.badges ?? [];
  const member = !!identity?.streamnook;

  return (
    <div className="min-w-0 flex-1">
      {/*
        Wrapping, not truncating, and deliberately NOT line-clamp: clamping
        needs `display: -webkit-box`, which fights the `background-clip: text`
        a 7TV paint is drawn with. `break-words` handles a long single-token
        name; the size step above keeps even a 25 character name to two lines.
      */}
      <div className="flex items-center gap-2 min-w-0">
        <div
          className={`font-bold text-textPrimary leading-tight break-words shrink-0 max-w-full ${nameSizeClass(displayName)}`}
          style={nameStyle}
        >
          {displayName}
        </div>
        {/* The applied 7TV paint by name, drawn in the paint itself, the same
            way the Cosmetics list previews one. It stays on the name's line and
            gives way first: a long paint name truncates, the name never does. */}
        {paint?.name && (
          <span className="glass-badge inline-flex items-center min-w-0 px-2 py-px rounded-full">
            <span
              className="text-[11px] font-bold truncate leading-snug"
              style={computePaintStyle(paint, '#9146FF')}
            >
              {paint.name}
            </span>
          </span>
        )}
      </div>

      {login && login.toLowerCase() !== displayName.toLowerCase() && (
        <div className="text-[13px] text-textMuted truncate">@{login}</div>
      )}

      {(badges.length > 0 || member) && (
        // Canonical badge order (utils/badgeOrder): Twitch, 7TV, the other chat
        // apps, then StreamNook last.
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {badges.map((b, i) => (
            <FallbackImage
              key={`${b.id}-${i}`}
              src={b.url}
              fallbackUrls={b.fallbacks}
              alt={b.title || b.name || ''}
              title={b.title || b.name}
              className="w-[18px] h-[18px] object-contain shrink-0"
            />
          ))}
          {member && (
            // Rust sends `streamnook` for members alone: StreamNookBadge has no
            // guard of its own and would hand a badge to everyone who is not one.
            // `side="bottom"` because this sits at the top of the viewport and
            // a popover growing upward clips off screen.
            <StreamNookBadge userId={userId} side="bottom" />
          )}
        </div>
      )}
    </div>
  );
};

export default OwnIdentityHeader;
