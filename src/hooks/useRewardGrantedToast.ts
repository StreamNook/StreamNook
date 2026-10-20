import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';

/**
 * Says so when Rust's reward claims (services/member_rewards.rs) grant the
 * member something new: a login-window or watch event reward, or a tenure
 * badge. Shown even when routine toasts are muted, since a new earn is not
 * routine. Granting only unlocks the look; the member applies it themselves.
 *
 * Mount ONCE, in the main window only: every window hears the event, and a
 * second listener would say it twice.
 */
export function useRewardGrantedToast(): void {
  useEffect(() => {
    const unlisten = listen<{ reward_id: string; title: string | null }>('reward-granted', (event) => {
      useAppStore
        .getState()
        .addToast(
          `Achievement unlocked: ${event.payload.title ?? 'event reward'}. Apply it from profile customization.`,
          'success',
          undefined,
          { alwaysShow: true },
        );
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);
}
