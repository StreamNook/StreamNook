// Android back chain for the IN-PLACE mobile shell (the adapted desktop App).
// MainActivity's OnBackPressedCallback evaluates window.__SN_BACK__(); a `true`
// return consumes the press, `false` lets native background the task (never
// kill it). The dedicated MobileApp shell overrides this with its own richer
// chain (navStore) when it mounts.
import { useAppStore } from '../stores/AppStore';

export function installInPlaceBackHandler(): void {
  const w = window as Window & { __SN_BACK__?: () => boolean };
  w.__SN_BACK__ = () => {
    const s = useAppStore.getState();
    if (s.showDropsOverlay) {
      s.setShowDropsOverlay(false);
      return true;
    }
    if (s.isSettingsOpen) {
      s.closeSettings();
      return true;
    }
    if (!s.isHomeActive) {
      // Back from the watch view returns Home; the stream keeps playing and
      // stays reachable through the Watch tab.
      s.toggleHome();
      return true;
    }
    if (s.homeActiveTab !== 'following') {
      s.navigateToHomeTab('following');
      return true;
    }
    return false;
  };
}
