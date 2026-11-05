// The earner the app uses: Rust saves through `grant_own_accolade`, the toast
// goes out through the store, and the announced set lives in localStorage. The
// rules are in clientAccolades.ts.
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { createEarner, storageAnnouncedStore } from './clientAccolades';

const storage = (): Storage | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

/** Ask Rust to save a client-attested accolade for the signed-in account, then say so once. */
export const earnClientAccolade = createEarner({
  userId: () => {
    const { isAuthenticated, currentUser } = useAppStore.getState();
    return isAuthenticated && currentUser?.user_id ? currentUser.user_id : null;
  },
  grant: (id) => invoke<boolean>('grant_own_accolade', { id }),
  announced: storageAnnouncedStore(storage()),
  toast: (text) => useAppStore.getState().addToast(text, 'success', undefined, { alwaysShow: true }),
});
