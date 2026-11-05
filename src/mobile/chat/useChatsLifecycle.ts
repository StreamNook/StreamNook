// The phone chats outside any one screen: the stream tab follows the player,
// the saved chats come back at launch, and with no player they leave their
// rooms after a while in the background (lifecycle's return joins them again,
// see chatResume).
//
// Mounted once, in the shell. The stream tab used to be pointed from inside the
// chat pane, which only exists while a pane is on screen; with the Chats screen
// there can be two panes or none, and the player closing must still retire the
// stream tab (a kept one stays as one of your chats).
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { streamProvider } from '../../utils/streamProvider';
import { useChatTabsStore } from './chatTabsStore';
import { watchLayerUp } from '../ui/coveredSurface';

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
  // room that happens to share the name. Each open (a tap, a VOD, the offline
  // room, the same channel again) puts its chat in front; a name or picture
  // arriving later only refreshes the tab.
  const streamProv = streamProvider(currentStream);
  const openSeq = useAppStore((s) => s.streamOpenSeq);
  const focusedSeq = useRef(openSeq);
  useEffect(() => {
    const opened = openSeq !== focusedSeq.current;
    focusedSeq.current = openSeq;
    syncStreamTab(
      currentStream?.user_login ?? null,
      currentStream?.user_id ?? null,
      currentStream?.user_name || currentStream?.user_login || '',
      currentStream?.profile_image_url ?? null,
      streamProv,
      opened,
    );
    // userId: another account's chats replace the list (and its stream tab),
    // so the stream tab is pointed again after the load above.
  }, [
    currentStream?.user_login,
    currentStream?.user_id,
    currentStream?.user_name,
    currentStream?.profile_image_url,
    streamProv,
    syncStreamTab,
    userId,
    openSeq,
  ]);

  // The room on screen, for the mentions hub: mentions there are read as they
  // land. Chat shows on the Chats tab (unless the full player covers it) and
  // under the full player; the mini player's chat is hidden.
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const activeTab = useMobileNavStore((s) => s.activeTab);
  const playerMode = useMobileNavStore((s) => s.playerMode);
  const streamUrl = useAppStore((s) => s.streamUrl);
  const [pageVisible, setPageVisible] = useState(() => document.visibilityState === 'visible');
  useEffect(() => {
    const on = () => setPageVisible(document.visibilityState === 'visible');
    document.addEventListener('visibilitychange', on);
    return () => document.removeEventListener('visibilitychange', on);
  }, []);
  // The watch layer's own rule: a stream still loading at full size covers the tabs too.
  const isLoading = useAppStore((s) => s.isLoading);
  const playerFull = watchLayerUp(playerMode, streamUrl, isLoading);
  const shown = pageVisible && !!activeChannel && (playerFull || activeTab === 'chats');
  const onScreen = shown ? activeChannel : null;
  useEffect(() => {
    void invoke('chat_tabs_visible', { rooms: onScreen ? [onScreen] : [] }).catch(() => {});
  }, [onScreen]);

  // Background: with no player, every room is left once Rust's park delay has
  // passed (Rust refuses while a stream tab exists, so audio-only playback
  // keeps its chat). Back in front the timer stops; the return itself (a
  // parked set joining again before any rebuild) is lifecycle's, in chatResume.
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
    };
    document.addEventListener('visibilitychange', onChange);
    return () => {
      document.removeEventListener('visibilitychange', onChange);
      if (timer) clearTimeout(timer);
    };
  }, []);
}
