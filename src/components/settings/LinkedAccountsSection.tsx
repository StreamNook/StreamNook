import { useEffect, useRef, useState } from 'react';
import { Loader2, UserPlus, Unlink, User, Check, Palette, LogOut, Star } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { addAccount, removeAccount, type StoredAccount } from '../../services/accountService';
import { useSendAccountStore } from '../../stores/sendAccountStore';
import AccountIdentityEditor from './AccountIdentityEditor';
import PlatformAccountRows from './PlatformAccountRows';
import { Tooltip } from '../ui/Tooltip';

/**
 * Manage linked Twitch accounts. Two distinct roles, deliberately kept separate
 * in the UI so they can't be confused:
 *
 *   • "Watching & streaming as" — the one account StreamNook is signed into.
 *     Everything you watch and stream runs through it. Changing it is a real
 *     switch (its token moves into the main slot and chat reconnects).
 *
 *   • "Send chat as" — which linked account your chat messages go out as. A
 *     one-click default that does NOT change who you watch as.
 */
export default function LinkedAccountsSection() {
  const addToast = useAppStore((s) => s.addToast);
  const setActiveAccount = useAppStore((s) => s.setActiveAccount);
  const signOutActiveAccount = useAppStore((s) => s.signOutActiveAccount);
  const closeSettings = useAppStore((s) => s.closeSettings);
  const accounts = useSendAccountStore((s) => s.accounts);
  const sendAsId = useSendAccountStore((s) => s.sendAsId);
  const setSendAsId = useSendAccountStore((s) => s.setSendAsId);

  const [adding, setAdding] = useState(false);
  const [removingId, setRemovingId] = useState<string | null>(null);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [editing, setEditing] = useState<StoredAccount | null>(null);
  const [signOutConfirm, setSignOutConfirm] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    void useSendAccountStore.getState().loadAccounts();
  }, []);

  const handleAdd = async () => {
    if (adding) return;
    setAdding(true);
    try {
      const account = await addAccount();
      addToast(`Linked @${account.login}`, 'success');
      await useSendAccountStore.getState().loadAccounts();
    } catch (e) {
      addToast(typeof e === 'string' ? e : 'Could not link account', 'error');
    } finally {
      if (mountedRef.current) setAdding(false);
    }
  };

  const handleRemove = async (userId: string, login: string) => {
    setRemovingId(userId);
    try {
      await removeAccount(userId);
      addToast(`Unlinked @${login}`, 'info');
      await useSendAccountStore.getState().loadAccounts();
    } catch (e) {
      addToast(typeof e === 'string' ? e : 'Could not unlink account', 'error');
    } finally {
      if (mountedRef.current) setRemovingId(null);
    }
  };

  const handleMakeMain = async (userId: string) => {
    if (switchingId) return;
    setSwitchingId(userId);
    try {
      // Re-establishes identity and reconnects chat on success (toast inside).
      await setActiveAccount(userId);
    } catch {
      // setActiveAccount surfaces its own error toast.
    } finally {
      if (mountedRef.current) setSwitchingId(null);
    }
  };

  const primary = accounts.find((a) => a.is_primary);
  const multiple = accounts.length >= 2;
  // The effective default sender: the explicit choice, else the main.
  const defaultId = sendAsId ?? primary?.user_id ?? null;
  const switching = switchingId !== null;

  const avatar = (account: StoredAccount) =>
    account.avatar_url ? (
      <img
        src={account.avatar_url}
        alt=""
        className="w-9 h-9 rounded-full object-cover flex-shrink-0"
      />
    ) : (
      <div className="w-9 h-9 rounded-full bg-white/5 flex items-center justify-center flex-shrink-0">
        <User size={17} className="text-textMuted" />
      </div>
    );

  return (
    // The id is a deep-link target: the title bar's platform switcher sends you
    // straight here when you pick a platform you have not connected yet, via
    // openSettings('Profile', 'settings-section-accounts').
    <div id="settings-section-accounts" className="glass-panel rounded-xl p-4 space-y-4">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide">Accounts</h4>
        <button
          onClick={handleAdd}
          disabled={adding}
          className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium glass-button disabled:cursor-wait disabled:opacity-70"
        >
          {adding ? (
            <>
              <Loader2 size={14} className="animate-spin" />
              Waiting for sign-in…
            </>
          ) : (
            <>
              <UserPlus size={14} />
              Add account
            </>
          )}
        </button>
      </div>

      {adding && (
        <p className="text-xs text-textSecondary">
          A browser window opened. Sign in as the account you want to add, then return here.
        </p>
      )}

      {/* ── Watching & streaming as: the one account you're signed into ─────── */}
      {primary && (
        <div className="space-y-1.5">
          <p className="text-[11px] font-medium uppercase tracking-wide text-textMuted">
            Watching &amp; streaming as
          </p>
          <div className="flex items-center gap-3 rounded-lg px-3 py-2.5 bg-white/[0.03]">
            {avatar(primary)}
            <div className="min-w-0 flex-1">
              <div className="text-sm text-textPrimary truncate">
                {primary.display_name || primary.login}
              </div>
              <div className="text-xs text-textSecondary truncate">@{primary.login}</div>
            </div>

            {signOutConfirm ? (
              <div className="flex items-center gap-1 flex-shrink-0">
                <button
                  onClick={() => setSignOutConfirm(false)}
                  className="text-[11px] text-textMuted hover:text-textPrimary px-2 py-1 rounded-md hover:bg-white/[0.05] transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={() => {
                    setSignOutConfirm(false);
                    signOutActiveAccount();
                    closeSettings();
                  }}
                  className="text-[11px] font-medium text-red-400 hover:bg-red-500/10 px-2 py-1 rounded-md transition-colors"
                >
                  Sign out
                </button>
              </div>
            ) : (
              <Tooltip content="Sign out">
                <button
                  onClick={() => setSignOutConfirm(true)}
                  aria-label="Sign out"
                  className="p-1.5 text-textMuted hover:text-red-400 hover:bg-red-500/10 rounded-md transition-colors flex-shrink-0"
                >
                  <LogOut size={15} />
                </button>
              </Tooltip>
            )}
          </div>
          <p className="text-xs text-textMuted">
            The account StreamNook is signed into. Everything you watch and stream runs through it.
            {multiple && ' Use “Make main” below to switch.'}
          </p>
        </div>
      )}

      {/* ── Send chat as: which linked account your messages go out as ──────── */}
      {multiple && (
        <div className="space-y-1.5">
          <p className="text-[11px] font-medium uppercase tracking-wide text-textMuted">
            Send chat as
          </p>

          {accounts.map((account) => {
            const isDefault = account.user_id === defaultId;
            const isMain = account.is_primary;
            return (
              <div
                key={account.user_id}
                className="flex items-center gap-3 rounded-lg px-3 py-2.5 bg-white/[0.03]"
              >
                {avatar(account)}

                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="text-sm text-textPrimary truncate">
                      {account.display_name || account.login}
                    </span>
                    {isMain && (
                      <span className="text-[10px] font-medium uppercase tracking-wide text-textMuted bg-white/5 px-1.5 py-0.5 rounded flex-shrink-0">
                        Main
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-textSecondary truncate">@{account.login}</div>
                </div>

                {/* Default-sender control. */}
                {isDefault ? (
                  <span className="flex items-center gap-1 text-[11px] font-medium text-accent flex-shrink-0">
                    <Check size={13} />
                    Default
                  </span>
                ) : (
                  <button
                    onClick={() => setSendAsId(account.user_id)}
                    className="text-[11px] text-textMuted hover:text-textPrimary px-2 py-1 rounded-md hover:bg-white/[0.05] transition-colors flex-shrink-0"
                  >
                    Set default
                  </button>
                )}

                {/* Secondary-only actions: make main, edit identity, unlink. */}
                {!isMain && (
                  <>
                    <Tooltip content="Make this the account you watch & stream as">
                      <button
                        onClick={() => handleMakeMain(account.user_id)}
                        disabled={switching}
                        aria-label={`Make @${account.login} your main`}
                        className="flex items-center gap-1 text-[11px] text-textMuted hover:text-textPrimary px-2 py-1 rounded-md hover:bg-white/[0.05] transition-colors disabled:cursor-wait disabled:opacity-60 flex-shrink-0"
                      >
                        {switchingId === account.user_id ? (
                          <Loader2 size={13} className="animate-spin" />
                        ) : (
                          <Star size={13} />
                        )}
                        Make main
                      </button>
                    </Tooltip>

                    <Tooltip content="Edit identity (7TV cosmetics + badges)">
                      <button
                        onClick={() => setEditing(account)}
                        aria-label={`Edit identity for @${account.login}`}
                        className="p-1.5 text-textMuted hover:text-textPrimary hover:bg-white/[0.05] rounded-md transition-colors flex-shrink-0"
                      >
                        <Palette size={15} />
                      </button>
                    </Tooltip>

                    <button
                      onClick={() => handleRemove(account.user_id, account.login)}
                      disabled={removingId === account.user_id}
                      aria-label={`Unlink @${account.login}`}
                      className="p-1.5 text-textMuted hover:text-red-400 hover:bg-red-500/10 rounded-md transition-colors disabled:cursor-wait flex-shrink-0"
                    >
                      {removingId === account.user_id ? (
                        <Loader2 size={15} className="animate-spin" />
                      ) : (
                        <Unlink size={15} />
                      )}
                    </button>
                  </>
                )}
              </div>
            );
          })}

          <p className="text-xs text-textMuted">
            Pick which account your chat messages go out as, or switch per message from the chat box.
            This doesn&apos;t change who you watch as.
          </p>
        </div>
      )}

      {!multiple && !adding && (
        <p className="text-xs text-textMuted">
          Add another account to send chat from it or switch which one you watch as.
        </p>
      )}

      {/* Kick and YouTube sit alongside Twitch rather than under it: three
          platforms, same weight, one place to manage all of them. */}
      <PlatformAccountRows />

      {editing && <AccountIdentityEditor account={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}
