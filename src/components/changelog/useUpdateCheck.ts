import { useState } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { checkNow, type UpdateCheckResult } from '../../services/updateStatus';
import { requestUpdateInstall } from '../../utils/updateEvents';

export interface PendingUpdate {
  current: string;
  latest: string;
  size: string | null;
  behind: number | null;
}

/**
 * The update check and install, as the changelog popup shows them.
 *
 * An update is known either from the automatic check (the store's `updateInfo`,
 * which also lights the title-bar pill) or from a check run here. A check run
 * here writes back to the store, so the pill and the popup never disagree.
 */
export function useUpdateCheck() {
  const updateInfo = useAppStore((s) => s.updateInfo);
  const setUpdateInfo = useAppStore((s) => s.setUpdateInfo);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<UpdateCheckResult | null>(null);
  // The install flow and its progress live in TitleBar, which puts up the
  // full-screen UpdateOverlay the moment it starts. This only acknowledges the
  // click until that happens.
  const [starting, setStarting] = useState(false);

  const runCheck = async () => {
    setChecking(true);
    const r = await checkNow();
    setUpdateInfo(
      r.kind === 'available'
        ? {
            current_version: r.status.current_version,
            latest_version: r.status.latest_version,
            releases_behind: r.behind,
          }
        : null,
    );
    setResult(r);
    setChecking(false);
    setStarting(false);
  };

  const install = () => {
    setStarting(true);
    // TitleBar ignores a second request while installing, so a double click
    // cannot start two installs.
    requestUpdateInstall();
  };

  let pending: PendingUpdate | null = null;
  if (result?.kind === 'available') {
    pending = {
      current: result.status.current_version,
      latest: result.status.latest_version,
      size: result.status.download_size ?? null,
      behind: result.behind,
    };
  } else if (updateInfo && result?.kind !== 'current') {
    pending = {
      current: updateInfo.current_version,
      latest: updateInfo.latest_version,
      size: null,
      behind: updateInfo.releases_behind ?? null,
    };
  }

  return {
    checking,
    starting,
    pending,
    upToDate: result?.kind === 'current',
    failure: result?.kind === 'failed' ? result.reason : null,
    runCheck,
    install,
  };
}
