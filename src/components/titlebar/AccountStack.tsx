// Who StreamNook is signed in as, at a glance, from the title bar.
//
// The Profile button grew into this: your Twitch picture stays the face, and
// when more than one platform account is connected their marks fan out beside
// it, so "Twitch and YouTube" is visible without opening Settings. Only the
// platform accounts you watch and chat as get a mark. The credentials that
// serve a feature (drops and channel points, 7TV) live in the flyout alone.
//
// Everything shown comes from the roster Rust builds (services/account_roster.rs)
// and pushes on change; nothing here decides who is connected.
//
// Hovering opens the flyout; clicking goes straight to Settings > Profile, as
// the Profile button always did. Connect flows are never run from here except
// the drops sign-in, which was already a one-click action in this bar.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { User } from 'lucide-react';
import { Package } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { useAccountRosterStore, type RosterAccount } from '../../stores/accountRosterStore';
import { ProviderMark } from '../ProviderLogo';
import { SevenTVLogo } from '../emotesets/SevenTVLogo';
import { providerLabel } from '../../types/providers';
import { deckGeometry } from '../../utils/markDeck';
import { Logger } from '../../utils/logger';

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
  const addToast = useAppStore((s) => s.addToast);
  // The picture already on screen before the roster's first read lands, so the
  // face never blinks to a placeholder at boot.
  const fallbackAvatar = useAppStore((s) => (s.isAuthenticated ? s.currentUser?.profile_image_url : null));

  const [open, setOpen] = useState(false);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(null);
  const [dropsSigningIn, setDropsSigningIn] = useState(false);
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

  // The drops sign-in reports its outcome as an event, not as the call's result.
  useEffect(() => {
    const uns: Array<() => void> = [];
    let disposed = false;
    const keep = (u: () => void) => (disposed ? u() : uns.push(u));
    void listen('drops-login-complete', () => {
      setDropsSigningIn(false);
      addToast('Signed in. Drops and channel points are on', 'success');
    }).then(keep);
    void listen<string>('drops-login-error', (e) => {
      setDropsSigningIn(false);
      Logger.error('[AccountStack] Drops login failed:', e.payload);
      addToast(`Drops sign-in failed: ${e.payload}`, 'error');
    }).then(keep);
    return () => {
      disposed = true;
      uns.forEach((u) => u());
    };
  }, [addToast]);

  const startDropsLogin = useCallback(async () => {
    if (dropsSigningIn) return;
    setDropsSigningIn(true);
    try {
      const url = await invoke<string>('start_drops_login');
      // Opened bound to the active account's web profile, so it reuses the
      // main login's twitch.tv session: authorize, no second sign-in.
      await invoke('open_drops_login_window', { url });
    } catch (e) {
      Logger.error('[AccountStack] Drops login failed:', e);
      addToast(`Drops sign-in failed: ${e instanceof Error ? e.message : String(e)}`, 'error');
      setDropsSigningIn(false);
    }
  }, [dropsSigningIn, addToast]);

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

  const main = roster?.main ?? [];
  const extras = roster?.extras ?? [];
  const twitch = main.find((a) => a.id === 'twitch');
  const connectedMain = main.filter((a) => a.status === 'connected');
  const twitchConnected = twitch ? twitch.status === 'connected' : !!fallbackAvatar;
  const avatar = twitch?.status === 'connected' ? twitch.avatar_url : fallbackAvatar;
  // The face is Twitch's; with Twitch signed out, the first connected
  // platform's picture stands in so the button still says who you are.
  const face = avatar ?? connectedMain[0]?.avatar_url ?? null;

  const drops = extras.find((a) => a.id === 'twitch_drops');
  const anyExpired = main.some((a) => a.status === 'expired');
  const dropsMissing = twitchConnected && drops?.status === 'not_connected';
  const needsAttention = anyExpired || dropsMissing;

  const tooltip = connectedMain.length
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
        onClick={goToProfile}
        aria-label={tooltip}
        aria-haspopup="menu"
        aria-expanded={open}
        className="titlebar-icon-btn relative gap-1.5"
      >
        {/* The attention dot rides the picture's corner, clear of the marks. */}
        <span className="relative flex">
          {face ? (
            <img src={face} alt="" className="h-[20px] w-[20px] rounded-full object-cover" draggable={false} />
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
                zIndex: 9999,
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
                  return (
                    <div key={a.id} role="menuitem" className="flex items-center gap-2.5 px-2.5 py-[6px]">
                      <ProviderMark provider={a.id} size={12} />
                      {connected && a.avatar_url ? (
                        <img
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
                      </span>
                      {expired ? (
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
                    const isDrops = a.id === 'twitch_drops';
                    return (
                      <div key={a.id} role="menuitem" className="flex items-center gap-2.5 px-2.5 py-[6px]">
                        <span className="flex h-3 w-3 flex-shrink-0 items-center justify-center text-textSecondary">
                          {isDrops ? <Package size={12} /> : <SevenTVLogo className="h-[9px] w-auto text-[#29b6f6]" />}
                        </span>
                        <span className="w-[18px] flex-shrink-0" aria-hidden />
                        <span className="min-w-0 flex-1 truncate text-[11.5px] leading-[15px] text-textSecondary">
                          {isDrops ? 'Drops & points' : '7TV'}
                        </span>
                        {connected ? (
                          <span className="flex flex-shrink-0 items-center gap-1.5 text-[10.5px] leading-none text-textMuted">
                            <StatusDot tone="ok" />
                            On
                          </span>
                        ) : isDrops ? (
                          // Drops ride the main Twitch login; without one there
                          // is nothing to authorize yet.
                          twitchConnected ? (
                            <>
                              <StatusDot tone="warn" />
                              <RowAction
                                label={dropsSigningIn ? 'Signing in…' : 'Sign in'}
                                onClick={() => void startDropsLogin()}
                                disabled={dropsSigningIn}
                              />
                            </>
                          ) : (
                            <span className="flex-shrink-0 text-[10.5px] leading-none text-textMuted">Off</span>
                          )
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
