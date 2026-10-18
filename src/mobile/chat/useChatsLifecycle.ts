// The phone chats outside any one screen: the stream tab follows the player,
// the saved chats come back at launch, and with no player they leave their
// rooms after a while in the background and join them again on return.
//
// Mounted once, in the shell. The stream tab used to be pointed from inside the
// chat pane, which only exists while a pane is on screen; with the Chats screen
// there can be two panes or none, and the player closing must still retire the
// stream tab (a kept one stays as one of your chats).
import { useEffect } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { streamProvider } from '../../utils/streamProvider';
import { useChatTabsStore } from './chatTabsStore';

export function useChatsLifecycle(): void {
  const currentStream = useAppStore((s) => s.currentStream);
  const syncStreamTab = useChatTabsStore((s) => s.syncStreamTab);
  const load = useChatTabsStore((s) => s.load);
  const userId = useAppStore((s) => s.currentUser?.user_id ?? null);

  // The saved chats, restored by Rust for whoever is signed in and not joined
  // until something looks. Read again after a sign-in or an account switch.
  useEffect(() => load(), [load, userId]);

  // Keep the stream-following tab pointed at whatever is playing, on the
  // platform it plays on: a Kick stream's room is `kick:slug`, never a Twitch
  // room that happens to share the name.
  const streamProv = streamProvider(currentStream);
  useEffect(() => {
    syncStreamTab(
      currentStream?.user_login ?? null,
      currentStream?.user_id ?? null,
      currentStream?.user_name || currentStream?.user_login || '',
      currentStream?.profile_image_url ?? null,
      streamProv,
    );
  }, [
    currentStream?.user_login,
    currentStream?.user_id,
    currentStream?.user_name,
    currentStream?.profile_image_url,
    streamProv,
    syncStreamTab,
  ]);

  // Background: with no player, every room is left once Rust's park delay has
  // passed (Rust refuses while a stream tab exists, so audio-only playback
  // keeps its chat). Back in front, a parked set joins again, and the chat
  // store's history load fills what was missed.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onChange = () => {
      const tabs = useChatTabsStore.getState();
      if (document.visibilityState === 'hidden') {
        if (timer || tabs.connection !== 'connected') return;
        timer = setTimeout(() => {
          timer = null;
          if (document.visibilityState === 'hidden') useChatTabsStore.getState().park();
        }, tabs.parkAfterSecs * 1000);
        return;
      }
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      if (tabs.connection === 'parked') tabs.connect();
    };
    document.addEventListener('visibilitychange', onChange);
    return () => {
      document.removeEventListener('visibilitychange', onChange);
      if (timer) clearTimeout(timer);
    };
  }, []);
}
