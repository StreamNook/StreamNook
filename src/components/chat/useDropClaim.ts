import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

/** Claims one drop, for chat's drops bar and the phones' lip sheet.
 *  "claimed" and "already_claimed" both mean it is yours, so both say so; the
 *  toast comes only from this click. */
export function useDropClaim() {
  const [claiming, setClaiming] = useState<string | null>(null);
  const claim = async (dropId: string, instanceId: string | null) => {
    if (claiming) return;
    setClaiming(dropId);
    try {
      await invoke<string>('claim_drop', { dropId, dropInstanceId: instanceId });
      useAppStore.getState().addToast('Drop claimed', 'success');
    } catch (err) {
      Logger.error('[DropsChatBar] claim failed:', err);
      useAppStore.getState().addToast('Could not claim that drop.', 'error');
    } finally {
      setClaiming(null);
    }
  };
  return { claiming, claim };
}
