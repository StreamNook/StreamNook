import { create } from 'zustand';

/**
 * Every account StreamNook is signed into, exactly as Rust built it
 * (`services/account_roster.rs`). Nothing here decides anything: the list,
 * its order and each row's status arrive ready to render, through
 * `get_account_roster` and the `account-roster-changed` event, both applied by
 * `platformAccountStore.applyRoster`.
 */

export type AccountStatus = 'connected' | 'expired' | 'not_connected';

/** Why an account has to sign in again, when StreamNook signed it out itself
 *  (`services/twitch_signin.rs`). */
export type SignInReason = 'new_permissions';

export type MainAccountId = 'twitch' | 'youtube' | 'kick' | 'tiktok';
export type ExtraAccountId = 'twitch_drops' | 'seventv';

export interface RosterAccount<Id extends string = string> {
  id: Id;
  status: AccountStatus;
  name: string | null;
  handle: string | null;
  avatar_url: string | null;
  account_id: string | null;
  /** Set only on an `expired` row StreamNook ended itself. Only Twitch carries
   *  one today. */
  reason?: SignInReason | null;
}

export interface AccountRoster {
  main: RosterAccount<MainAccountId>[];
  extras: RosterAccount<ExtraAccountId>[];
}

interface AccountRosterStore {
  /** Null until the first read lands. */
  roster: AccountRoster | null;
  setRoster: (roster: AccountRoster) => void;
}

export const useAccountRosterStore = create<AccountRosterStore>((set) => ({
  roster: null,
  setRoster: (roster) => set({ roster }),
}));
