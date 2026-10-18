// Lightweight quick-add modal shown when a 7TV emote is left-clicked in chat.
// It is standalone: it does NOT open the full 7TV Emotes overlay. It loads the
// channels you can edit for its cross-channel add picker. If you're not
// connected to 7TV, the modal shows a Connect button that opens the 7TV login
// and reloads once you're signed in.
import { useCallback, useEffect, useState } from 'react';
import { useEmoteCardStore } from '../stores/emoteCardStore';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import EmoteDetail from './emotesets/EmoteDetail';
import { getEditableChannels, invalidateEditableChannels, type EditableChannel } from '../services/seventvEditorService';

export default function EmoteSpotlight() {
  // Tell chat this window has the 7TV spotlight; windows without it open the
  // plain emote card for 7TV emotes instead of doing nothing.
  useEffect(() => {
    useEmoteCardStore.getState().setSeventvSpotlight(true);
    return () => useEmoteCardStore.getState().setSeventvSpotlight(false);
  }, []);
  const spotlight = useAppStore((s) => s.emoteSpotlight);
  const setSpotlight = useAppStore((s) => s.setEmoteSpotlight);
  const addToast = useAppStore((s) => s.addToast);

  const [channels, setChannels] = useState<EditableChannel[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const status = (await invoke('get_seventv_auth_status')) as { is_authenticated: boolean };
      if (!status?.is_authenticated) {
        setChannels([]);
        return;
      }
      const list = await getEditableChannels(undefined, true);
      setChannels(list.filter((c) => c.inviteState === 'ACCEPTED' && c.perms.manageEmotes));
    } catch {
      // Token rejected / network error: treat as not connected. The modal shows
      // the Connect button and the emote info either way.
      setChannels([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!spotlight) return;
    setChannels([]);
    load();
  }, [spotlight, load]);

  // Reload when a 7TV login completes (the Connect button opens that window).
  useEffect(() => {
    if (!spotlight) return;
    let un: (() => void) | undefined;
    let disposed = false;
    listen('seventv-connected', () => {
      invalidateEditableChannels();
      load();
    }).then((u) => {
      if (disposed) {
        u();
        return;
      }
      un = u;
    });
    return () => {
      disposed = true;
      un?.();
    };
  }, [spotlight, load]);

  const connect = useCallback(async () => {
    try {
      await invoke('open_seventv_login_window');
    } catch (e) {
      addToast(e instanceof Error ? e.message : '7TV login failed to open', 'error');
    }
  }, [addToast]);

  if (!spotlight) return null;

  return (
    <EmoteDetail
      key={spotlight.id}
      ctx={{ emoteId: spotlight.id, defaultName: spotlight.name }}
      canManage
      channels={channels}
      channelsLoading={loading}
      onConnect={connect}
      onClose={() => setSpotlight(null)}
    />
  );
}
