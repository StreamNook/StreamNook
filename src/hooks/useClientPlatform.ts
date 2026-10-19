import { useEffect, useState } from 'react';
import { getClientIdentity, getClientIdentitySync } from '../utils/appVersion';
import { IS_ANDROID, IS_LINUX, IS_MAC, IS_MOBILE } from '../utils/platform';
import { Logger } from '../utils/logger';
import type { PlatformKey } from '../components/ui/PlatformLogo';

export interface PlatformEntry {
  key: PlatformKey;
  name: string;
}

/** Every platform StreamNook ships on, in the downloads page's order. */
export const PLATFORMS: PlatformEntry[] = [
  { key: 'windows', name: 'Windows' },
  { key: 'macos', name: 'macOS' },
  { key: 'linux', name: 'Linux' },
  { key: 'android', name: 'Android' },
  { key: 'ios', name: 'iPhone' },
];

/** Where every platform's build is offered. The page has one card per platform. */
export const DOWNLOADS_URL = 'https://streamnook.app/downloads';

const isPlatformKey = (value: string): value is PlatformKey => PLATFORMS.some((p) => p.key === value);

// Rust's answer when it is already cached (it is primed at boot), else the
// synchronous UA flags, so the first paint already names the right platform
// and nothing swaps in a tick later.
const initialPlatform = (): PlatformKey => {
  const reported = getClientIdentitySync()?.platform;
  if (reported && isPlatformKey(reported)) return reported;
  if (IS_ANDROID) return 'android';
  if (IS_MOBILE) return 'ios';
  return IS_MAC ? 'macos' : IS_LINUX ? 'linux' : 'windows';
};

/**
 * The platform this build is (from Rust's `ClientIdentity`), its version, and
 * the platforms it is not, for the "also on your other devices" links.
 */
export function useClientPlatform() {
  const [platform, setPlatform] = useState<PlatformKey>(initialPlatform);
  const [version, setVersion] = useState(() => getClientIdentitySync()?.app_version ?? '');

  useEffect(() => {
    getClientIdentity()
      .then((id) => {
        setVersion(id.app_version);
        if (isPlatformKey(id.platform)) setPlatform(id.platform);
      })
      .catch((err) => Logger.error('Failed to get client identity:', err));
  }, []);

  const current = PLATFORMS.find((p) => p.key === platform) ?? PLATFORMS[0];
  const others = PLATFORMS.filter((p) => p.key !== platform);
  return { current, others, version };
}
