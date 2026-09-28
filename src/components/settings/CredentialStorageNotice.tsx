import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, CircleHelp } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';

/** Mirrors `CredentialStorage` in src-tauri/src/services/token_vault.rs. */
interface CredentialStorage {
  /** True when no system keyring answered, so the key that encrypts sign-ins
   *  is kept in a file beside them. */
  key_on_disk: boolean;
  /** What this platform's keyring is called, for the explanation. */
  system_store: string;
}

/**
 * One line under the accounts, shown only when sign-ins are not protected by
 * the system keyring (a Linux session without a Secret Service, a denied
 * Keychain request). Rust decides; this only presents it.
 */
export default function CredentialStorageNotice() {
  const [storage, setStorage] = useState<CredentialStorage | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<CredentialStorage>('get_credential_storage')
      .then((s) => {
        if (alive) setStorage(s);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  if (!storage?.key_on_disk) return null;

  return (
    <div className="flex items-start gap-1.5 text-xs text-amber-300">
      <AlertTriangle size={12} className="mt-0.5 flex-shrink-0" />
      <span>
        Your sign-ins on this device are not protected by a keyring.
        <Tooltip
          content={
            <span className="block max-w-[36ch] text-left leading-relaxed">
              StreamNook could not reach {storage.system_store}, so the key that encrypts your
              sign-ins is saved next to them, where anything that can read your files can use it.
              Once it is running and unlocked, restart StreamNook and your sign-ins move to it on
              their own.
            </span>
          }
        >
          <span
            tabIndex={0}
            aria-label="Why sign-ins are not protected"
            className="ml-1.5 inline-flex align-middle text-textMuted hover:text-textSecondary transition-colors cursor-help"
          >
            <CircleHelp size={12} />
          </span>
        </Tooltip>
      </span>
    </div>
  );
}
