// Settings > Stream Overlay: the shared overlay builder, hosted by the desktop app.
//
// The builder itself (components/overlay/builder) is the same component
// streamnook.app/overlays runs. This file supplies what only the app can: Rust
// makes the overlays API calls with its own copy of the Twitch token
// (src-tauri/src/commands/streamnook_api.rs, streamnook_api_request), so the page
// never holds it; channel lookups go through Rust; and the live preview reads
// the app's own chat pipeline.

import { useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { LiveOverlayFeed } from '../overlay/LiveOverlayFeed';
import { OverlayBuilder } from '../overlay/builder/OverlayBuilder';
import { OverlayHostProvider, type OverlayApiResponse, type OverlayHost } from '../overlay/builder/overlayHost';
import { lookupError, resolveKickSlug, resolveYouTubeIdentifier, youTubeChannelTitle } from '../../services/channelLookup';

interface ApiResult {
  status: number;
  ok: boolean;
  body: string;
}

async function request(
  method: 'GET' | 'POST' | 'DELETE',
  path: string,
  query?: string,
  body?: unknown,
): Promise<OverlayApiResponse> {
  let r: ApiResult;
  try {
    r = await invoke<ApiResult>('streamnook_api_request', { method, path, query, body });
  } catch (e) {
    // Rust refuses before the network when there is no Twitch session.
    if (String(e).startsWith('no_token')) throw new Error('Sign in to Twitch in StreamNook to publish an overlay.');
    throw e;
  }
  return {
    ok: r.ok,
    status: r.status,
    json<T>(): T | null {
      try {
        return JSON.parse(r.body) as T;
      } catch {
        return null;
      }
    },
  };
}

const OverlaySettings = () => {
  const accountId = useAppStore((s) => s.currentUser?.user_id ?? null);
  const loginToTwitch = useAppStore((s) => s.loginToTwitch);
  const host = useMemo<OverlayHost>(
    () => ({
      accountId,
      signIn: () => void loginToTwitch(),
      request,
      resolveYouTubeIdentifier,
      resolveKickSlug,
      youTubeChannelTitle,
      lookupError,
      LivePreview: LiveOverlayFeed,
    }),
    [accountId, loginToTwitch],
  );
  return (
    <OverlayHostProvider value={host}>
      <OverlayBuilder />
    </OverlayHostProvider>
  );
};

export default OverlaySettings;
