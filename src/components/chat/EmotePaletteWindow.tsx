// The emote menu popped out into a window of its own (`#/emote-palette`), so it
// can stay open beside the chat for quick reactions. It is the same menu the
// chat box opens (EmotePickerPanel). Rust decides which chat box it types into
// and gives that window focus back after every pick, so Enter sends
// (commands/emote_palette.rs); this page only shows the menu for that chat.
//
// It never joins the chat: the emote set comes from Rust's shared copy with
// disk-cache paths already on it, and nothing here listens to messages.
import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Pin, PinOff, X } from 'lucide-react';
import { EmotePickerPanel } from './EmotePickerPanel';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import { useChannelEmotes } from '../../stores/chatConnectionStore';
import { useThemeBoot } from '../../boot/useThemeBoot';
import { useEmoteOwnerNames } from '../../hooks/useEmoteOwnerNames';
import type { PaletteChannel } from '../../hooks/useEmotePalette';
import {
  youTubeChannelEmojiRows,
  type Emote,
  type EmoteSet,
  type YouTubeChannelEmoji,
} from '../../services/emoteService';
import { listenForSettingsUpdates } from '../../utils/settingsBroadcast';
import { Logger } from '../../utils/logger';

const EMPTY_SET: EmoteSet = { twitch: [], bttv: [], '7tv': [], ffz: [], kick: [], youtube: [] };
// Fills the window under the titlebar instead of floating above a chat box.
const PANEL_CLASS = 'flex min-h-0 flex-1 flex-col overflow-hidden';

export default function EmotePaletteWindow() {
  useThemeBoot();
  const [channel, setChannel] = useState<PaletteChannel | null>(null);
  const [pinned, setPinned] = useState(false);
  const [youTubeRows, setYouTubeRows] = useState<{ key: string; rows: Emote[] } | null>(null);

  // This window's store boots empty: settings for the theme and the menu's
  // own preferences, kept fresh when another window saves; and whether the
  // viewer may use FFZ subscriber effects (the only account fact the menu reads).
  useEffect(() => {
    void useAppStore.getState().loadSettings().catch((err) => Logger.warn('[EmotePalette] loadSettings failed:', err));
    void invoke<{ is_subwoofer: boolean }>('ffz_local_user_status')
      .then((s) => useAppStore.setState({ ffzIsSubwoofer: !!s?.is_subwoofer }))
      .catch(() => {});
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listenForSettingsUpdates(() => {
      void useAppStore.getState().loadSettings();
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // The chat this menu serves: asked once, then told whenever it changes.
  // Listening first means a change landing during the ask is never missed.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWindow()
      .listen<PaletteChannel>('emote-palette://target', (e) => setChannel(e.payload))
      .then((u) => {
        if (cancelled) {
          u();
          return;
        }
        unlisten = u;
        return invoke<PaletteChannel | null>('get_emote_palette_target').then((c) => {
          if (!cancelled && c) setChannel(c);
        });
      })
      .catch((err) => Logger.warn('[EmotePalette] could not read its chat:', err));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const provider = channel?.provider ?? 'twitch';
  const baseEmotes = useChannelEmotes(channel?.login ?? null, channel?.id ?? null, provider);

  // YouTube's own channel emoji live apart from the 7TV set (Rust has them
  // from the chat page), so a YouTube chat's menu asks for them by video id.
  const youTubeKey = provider === 'youtube' ? (channel?.login ?? '') : '';
  useEffect(() => {
    if (!youTubeKey) return;
    let cancelled = false;
    void invoke<YouTubeChannelEmoji[]>('get_youtube_channel_emojis', { channel: youTubeKey })
      .then((list) => {
        if (!cancelled) setYouTubeRows({ key: youTubeKey, rows: youTubeChannelEmojiRows(list ?? []) });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [youTubeKey]);

  const emotes = useMemo(() => {
    if (provider !== 'youtube') return baseEmotes;
    return {
      ...(baseEmotes ?? EMPTY_SET),
      youtube: youTubeRows?.key === youTubeKey ? youTubeRows.rows : [],
    };
  }, [baseEmotes, provider, youTubeRows, youTubeKey]);
  const channelNameCache = useEmoteOwnerNames(emotes);

  const insert = useCallback((text: string) => {
    invoke('emote_palette_insert', { text }).catch((err) => Logger.warn('[EmotePalette] insert failed:', err));
  }, []);
  // The menu stays open between picks; a GIF send (which closes the in-chat
  // menu) leaves it open too.
  const keepOpen = useCallback(() => {}, []);

  const close = () => void getCurrentWindow().close();
  const togglePin = async () => {
    try {
      await getCurrentWindow().setAlwaysOnTop(!pinned);
      setPinned(!pinned);
    } catch (err) {
      Logger.warn('[EmotePalette] setAlwaysOnTop failed:', err);
    }
  };

  // Escape closes it, unless it is clearing the search box.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      const el = e.target as HTMLInputElement | null;
      if (el?.tagName === 'INPUT' && el.value) return;
      void getCurrentWindow().close();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const title = channel?.name || channel?.login || 'Emotes';

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-textPrimary">
      <div
        data-tauri-drag-region
        className="relative z-50 flex h-[33px] shrink-0 select-none items-center justify-between border-b border-borderSubtle bg-secondary px-3"
      >
        <span data-tauri-drag-region className="pointer-events-none truncate text-xs font-semibold tracking-wide text-textSecondary">
          {title}
        </span>
        <div className="flex space-x-1" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <Tooltip content={pinned ? 'Unpin from top' : 'Keep on top'} delay={200}>
            <button
              type="button"
              onClick={() => void togglePin()}
              data-tauri-drag-region="false"
              aria-pressed={pinned}
              className={`rounded p-1.5 transition-all duration-200 ${
                pinned ? 'text-accent' : 'text-textSecondary hover:text-textPrimary'
              }`}
            >
              {pinned ? <PinOff size={14} /> : <Pin size={14} />}
            </button>
          </Tooltip>
          <Tooltip content="Close" delay={200}>
            <button
              type="button"
              onClick={close}
              data-tauri-drag-region="false"
              className="rounded p-1.5 text-textSecondary transition-all duration-200 hover:text-red-400"
            >
              <X size={14} />
            </button>
          </Tooltip>
        </div>
      </div>
      {channel && (
        <EmotePickerPanel
          // A different chat starts on its own default tab.
          key={`${provider}:${channel.login}`}
          open
          onClose={keepOpen}
          emotes={emotes}
          isTwitch={provider === 'twitch'}
          isKick={provider === 'kick'}
          isYouTube={provider === 'youtube'}
          channelId={channel.id ?? undefined}
          channelLogin={channel.login}
          isLoadingEmotes={!emotes}
          channelNameCache={channelNameCache}
          onInsert={insert}
          className={PANEL_CLASS}
        />
      )}
    </div>
  );
}
