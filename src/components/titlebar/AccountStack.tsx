// Who StreamNook is signed in as, at a glance, from the title bar.
//
// The Profile button grew into this: your Twitch picture stays the face, and
// when more than one platform account is connected their marks fan out beside
// it, so "Twitch and YouTube" is visible without opening Settings. Only the
// platform accounts you watch and chat as get a mark. 7TV, a credential that
// serves a feature, lives in the flyout alone. The Drops approval is not shown
// at all: Twitch sign-in asks for it, and whatever needs it asks in place.
//
// Everything shown comes from the roster Rust builds (services/account_roster.rs)
// and pushes on change; nothing here decides who is connected.
//
// Hovering opens the flyout; clicking goes straight to Settings > Profile, as
// the Profile button always did. Connect flows are never run from here except
// the Twitch sign-in StreamNook itself asked for: while Rust holds a reason for
// it, the button says "Sign in again" and a click starts the normal sign-in.

import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { User } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { useAccountRosterStore, type RosterAccount } from '../../stores/accountRosterStore';
import { ProviderMark } from '../ProviderLogo';
import { SevenTVLogo } from '../emotesets/SevenTVLogo';
import { providerLabel } from '../../types/providers';
import { deckGeometry } from '../../utils/markDeck';
import { signInReasonText, twitchSignInReason } from '../../utils/twitchSignIn';
import { SevenTvAvatar } from '../SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';

/** A roster account's cosmetics id: its `account_id` is the id that
 *  platform's chat stamps on its messages, the space 7TV keys chatters in. */
const accountSevenTvId = (a: RosterAccount | undefined): string | null =>
  a ? sevenTvIdForStream({ provider: a.id, user_id: a.account_id }) : null;

const FLYOUT_WIDTH = 252;

/** The connected platforms' marks, stepped like the "All platforms" deck in
 *  the platform switcher so the two read as one family. */
function ConnectedDeck({ accounts }: { accounts: RosterAccount[] }) {
  const size = 16;
  const { card, glyph, dx, dy, spread, height } = deckGeometry(size, accounts.length);
  return (
    <span className="relative block flex-shrink-0" style={{ width: spread, height }} aria-hidden>
      {accounts.map((a, i) => (
        <span
          key={a.id}
          className="absolute flex items-center justify-center"
          style={{ left: dx * i, top: dy * i, width: card, height: card, zIndex: i + 1 }}
        >
          <ProviderMark provider={a.id as 'twitch'} size={glyph} />
        </span>
      ))}
    </span>
  );
}

function StatusDot({ tone }: { tone: 'ok' | 'warn' }) {
  return (
    <span
      className={`h-1.5 w-1.5 flex-shrink-0 rounded-full ${tone === 'ok' ? 'bg-green-400' : 'bg-amber-400'}`}
      aria-hidden
    />
  );
}

function RowAction({ label, onClick, disabled }: { label: string; onClick: () => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="ml-auto flex-shrink-0 rounded-md px-1.5 py-1 text-[10.5px] leading-none text-textMuted outline-none transition-colors duration-150 hover:bg-white/[0.07] hover:text-textPrimary focus-visible:bg-white/[0.07] disabled:opacity-60"
    >
      {label}
    </button>
  );
}

export default function AccountStack() {
  const roster = useAccountRosterStore((s) => s.roster);
  const openSettings = useAppStore((s) => s.openSettings);
  const loginToTwitch = useAppStore((s) => s.loginToTwitch);
  // The picture already on screen before the roster's first read lands, so the
  // face never blinks to a placeholder at boot.
  const fallbackAvatar = useAppStore((s) => (s.isAuthenticated ? s.currentUser?.profile_image_url : null));
  const fallbackId = useAppStore((s) => (s.isAuthenticated ? s.currentUser?.user_id : null));

  const [open, setOpen] = useState(false);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(null);
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancelClose = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = null;
  };
  // Grace period for crossing the gap between the anchor and the flyout.
  const scheduleClose = () => {
    cancelClose();
    closeTimer.current = setTimeout(() => setOpen(false), 180);
  };
  useEffect(() => cancelClose, []);

  // Portalled to <body> to escape the title bar's stacking context, hung from
  // the anchor's right edge since this sits at the right of the bar.
  useLayoutEffect(() => {
    if (!open) return;
    const reposition = () => {
      const el = anchorRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const left = Math.max(8, Math.round(r.right - FLYOUT_WIDTH));
      setMenuPos({ top: Math.round(r.bottom + 8), left });
    };
    reposition();
    window.addEventListener('resize', reposition);
    return () => window.removeEventListener('resize', reposition);
  }, [open]);

  const goToAccounts = () => {
    cancelClose();
    setOpen(false);
    openSettings('Profile', 'settings-section-accounts');
  };
  const goToProfile = () => {
    cancelClose();
    setOpen(false);
    openSettings('Profile');
  };
  // The same sign-in Home and Settings run, nothing extra.
  const signInToTwitch = () => {
    cancelClose();
    setOpen(false);
    void loginToTwitch();
  };

  const main = roster?.main ?? [];
  // The Drops approval rides Twitch sign-in and is asked for in place, so it
  // is never a row of its own.
  const extras = (roster?.extras ?? []).filter((a) => a.id !== 'twitch_drops');
  const twitch = main.find((a) => a.id === 'twitch');
  const connectedMain = main.filter((a) => a.status === 'connected');
  const avatar = twitch?.status === 'connected' ? twitch.avatar_url : fallbackAvatar;
  // StreamNook signed you out of Twitch itself and Rust says why.
  const signInReason = twitchSignInReason(roster);
  // The face is Twitch's; with Twitch signed out, the first connected
  // platform's picture stands in so the button still says who you are. Not
  // while it says "Sign in again": that is about Twitch, and another
  // platform's face beside it would point at the wrong account.
  const face = signInReason ? null : (avatar ?? connectedMain[0]?.avatar_url ?? null);
  const faceSevenTvId = signInReason
    ? null
    : avatar
      ? twitch?.status === 'connected'
        ? accountSevenTvId(twitch)
        : fallbackId
      : accountSevenTvId(connectedMain[0]);

  const needsAttention = main.some((a) => a.status === 'expired');

  const tooltip = signInReason
    ? signInReasonText(signInReason)
    : connectedMain.length
      ? `Signed in to ${connectedMain.map((a) => providerLabel(a.id)).join(', ')}`
      : 'Sign in';

  return (
    <div
      className="relative"
      onMouseEnter={() => {
        cancelClose();
        setOpen(true);
      }}
      onMouseLeave={scheduleClose}
      onFocus={() => {
        cancelClose();
        setOpen(true);
      }}
      onBlur={(e) => {
        const t = e.relatedTarget as Node | null;
        if (!(anchorRef.current?.contains(t) || menuRef.current?.contains(t))) setOpen(false);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && open) {
          setOpen(false);
          anchorRef.current?.focus();
          e.stopPropagation();
        }
      }}
    >
      <button
        ref={anchorRef}
        type="button"
        onClick={signInReason ? signInToTwitch : goToProfile}
        aria-label={tooltip}
        aria-haspopup="menu"
        aria-expanded={open}
        className="titlebar-icon-btn relative gap-1.5"
      >
        {/* The attention dot rides the picture's corner, clear of the marks. */}
        <span className="relative flex">
          {face ? (
            <SevenTvAvatar
              cosmeticsId={faceSevenTvId}
              px={20}
              src={face}
              alt=""
              className="h-[20px] w-[20px] rounded-full object-cover"
              draggable={false}
            />
          ) : (
            <User size={20} />
          )}
          {needsAttention && (
            <span
              className="absolute -bottom-px -right-px h-[7px] w-[7px] rounded-full bg-amber-400 ring-[1.5px] ring-[var(--color-background)]"
              aria-hidden
            />
          )}
        </span>
        {/* The amber dot plus words: a status, not a chip. The words say what
            a click does, so a click does exactly that. */}
        {signInReason && (
          <span className="whitespace-nowrap pr-0.5 text-[11px] font-medium leading-none">Sign in again</span>
        )}
        {/* Marks only where platforms mix: one account needs no logo to say
            which platform it is. */}
        {connectedMain.length > 1 && <ConnectedDeck accounts={connectedMain} />}
      </button>

      {createPortal(
        <AnimatePresence>
          {open && menuPos && (
            <motion.div
              ref={menuRef}
              role="menu"
              aria-label="Accounts"
              className="glass-flyout overflow-hidden"
              style={{
                position: 'fixed',
                top: menuPos.top,
                left: menuPos.left,
                width: FLYOUT_WIDTH,
                zIndex: 'var(--sn-z-titlebar-menu)',
                transformOrigin: 'top right',
              }}
              onMouseEnter={cancelClose}
              onMouseLeave={scheduleClose}
              initial={{ opacity: 0, scale: 0.96, y: -5 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.97, y: -4, transition: { duration: 0.12, ease: 'easeIn' } }}
              // The platform switcher's spring, so the two flyouts move alike.
              transition={{
                type: 'spring',
                stiffness: 360,
                damping: 32,
                mass: 0.9,
                opacity: { duration: 0.15, ease: 'easeOut' },
              }}
            >
              <div className="py-1">
                {main.map((a) => {
                  const connected = a.status === 'connected';
                  const expired = a.status === 'expired';
                  const label = providerLabel(a.id);
                  const owed = a.id === 'twitch' ? signInReason : null;
                  return (
                    <div key={a.id} role="menuitem" className="flex items-center gap-2.5 px-2.5 py-[6px]">
                      <ProviderMark provider={a.id} size={12} />
                      {connected && a.avatar_url ? (
                        <SevenTvAvatar
                          cosmeticsId={accountSevenTvId(a)}
                          px={18}
                          src={a.avatar_url}
                          alt=""
                          className="h-[18px] w-[18px] flex-shrink-0 rounded-full object-cover"
                          draggable={false}
                        />
                      ) : (
                        // Holds the picture's column so every name lines up.
                        <span className="w-[18px] flex-shrink-0" aria-hidden />
                      )}
                      <span className="min-w-0 flex-1">
                        <span
                          className={`block truncate text-[11.5px] leading-[15px] ${
                            connected ? 'text-textPrimary' : 'text-textSecondary'
                          }`}
                        >
                          {connected ? (a.name ?? label) : label}
                        </span>
                        {connected && a.handle && a.handle.toLowerCase() !== a.name?.toLowerCase() && (
                          <span className="block truncate text-[10.5px] leading-[14px] text-textMuted">@{a.handle}</span>
                        )}
                        {/* Why, under the name it is about. Wraps rather than
                            truncates: the reason is the point of the row. */}
                        {owed && (
                          <span className="mt-[2px] flex items-start gap-1.5 text-[10.5px] leading-[14px] text-textMuted">
                            <span className="mt-[4px] flex flex-shrink-0">
                              <StatusDot tone="warn" />
                            </span>
                            {signInReasonText(owed, 'short')}
                          </span>
                        )}
                      </span>
                      {owed ? (
                        <RowAction label="Sign in" onClick={signInToTwitch} />
                      ) : expired ? (
                        <>
                          <span className="flex flex-shrink-0 items-center gap-1.5 text-[10.5px] leading-none text-textMuted">
                            <StatusDot tone="warn" />
                            Expired
                          </span>
                          <RowAction label="Reconnect" onClick={goToAccounts} />
                        </>
                      ) : !connected ? (
                        <RowAction label={a.id === 'twitch' ? 'Sign in' : 'Connect'} onClick={a.id === 'twitch' ? goToProfile : goToAccounts} />
                      ) : null}
                    </div>
                  );
                })}
              </div>

              {extras.length > 0 && (
                <div
                  className="py-1"
                  style={{ borderTop: '1px solid color-mix(in srgb, var(--color-border-subtle) 55%, transparent)' }}
                >
                  {extras.map((a) => {
                    const connected = a.status === 'connected';
                    return (
                      <div key={a.id} role="menuitem" className="flex items-center gap-2.5 px-2.5 py-[6px]">
                        <span className="flex h-3 w-3 flex-shrink-0 items-center justify-center text-textSecondary">
                          <SevenTVLogo className="h-[9px] w-auto text-[#29b6f6]" />
                        </span>
                        <span className="w-[18px] flex-shrink-0" aria-hidden />
                        <span className="min-w-0 flex-1 truncate text-[11.5px] leading-[15px] text-textSecondary">
                          7TV
                        </span>
                        {connected ? (
                          <span className="flex flex-shrink-0 items-center gap-1.5 text-[10.5px] leading-none text-textMuted">
                            <StatusDot tone="ok" />
                            On
                          </span>
                        ) : (
                          <RowAction label="Connect" onClick={goToProfile} />
                        )}
                      </div>
                    );
                  })}
                </div>
              )}

              <button
                type="button"
                onClick={goToAccounts}
                className="flex w-full items-center px-3 py-2 text-left text-[10.5px] leading-[14px] text-textMuted outline-none transition-colors duration-150 hover:bg-white/[0.05] hover:text-textPrimary focus-visible:bg-white/[0.05]"
                style={{ borderTop: '1px solid color-mix(in srgb, var(--color-border-subtle) 55%, transparent)' }}
              >
                Manage accounts
              </button>
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </div>
  );
}
