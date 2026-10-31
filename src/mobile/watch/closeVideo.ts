// Close video (SN-37) wired to the app: the rule and the order are in
// closeVideoRule.ts.
import { useAppStore } from '../../stores/AppStore';
import { makeKey } from '../../utils/providerKey';
import { streamProvider } from '../../utils/streamProvider';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { useMobileNavStore } from '../navStore';
import { closeVideoRunner, miniCloseKeepsChat, tabIsStream, type CloseVideoDeps } from './closeVideoRule';

/** The stream's own tab, as the mirror has it. */
function streamTab(): { channel: string; kept: boolean } | null {
  const stream = useAppStore.getState().currentStream;
  const tab = useChatTabsStore.getState().tabs.find((t) => t.pinnedToStream && tabIsStream(t, stream));
  return tab ? { channel: tab.channel, kept: tab.kept } : null;
}

function liveDeps(): CloseVideoDeps {
  return {
    playing: () => {
      const s = useAppStore.getState();
      if (!s.currentStream || !(s.streamUrl || s.isLoading)) return null;
      return makeKey(streamProvider(s.currentStream), s.currentStream.user_login);
    },
    streamTab,
    keep: (channel) => useChatTabsStore.getState().keep(channel, true),
    forget: (channel) => useChatTabsStore.getState().removeTab(channel),
    toChats: () => useMobileNavStore.getState().setTab('chats'),
    close: () => useAppStore.getState().exitStream(),
    failed: () => useAppStore.getState().addToast('Could not open this chat in Chats.', 'error'),
  };
}

/** The header's and the error card's X (`true`), or the mini player's close on
 *  Chats (`false`, already there). */
export const closeVideo = closeVideoRunner(liveDeps);

/** Whether the mini player's close should keep the room (Chats in front showing it). */
export function miniCloseKeepsChatNow(): boolean {
  return miniCloseKeepsChat({
    onChats: useMobileNavStore.getState().activeTab === 'chats',
    activeChannel: useChatTabsStore.getState().activeChannel,
    streamTab: streamTab(),
  });
}
