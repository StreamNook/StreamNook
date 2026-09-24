// What the overlay builder needs from wherever it runs.
//
// The same builder runs in the desktop app (Settings > Stream Overlay) and on
// streamnook.app/overlays, synced one way from here like the renderer. The few
// things that differ between the two are behind this interface: how a request
// reaches the overlays API (Rust holds the Twitch token on desktop; the site uses
// its session cookie), how a channel link is looked up, and which feed drives the
// live preview. Nothing in the builder imports Tauri; a test keeps it that way.

import { createContext, useContext, type ComponentType } from 'react';
import type { OverlayStyle } from '../overlayConfig';
import type { ProviderId } from '../../../types/providers';

export interface OverlaySourceRef {
  provider: ProviderId;
  channel: string;
}

export interface OverlayApiResponse {
  ok: boolean;
  status: number;
  json<T>(): T | null;
}

export interface OverlayHost {
  /** The signed-in Twitch account's id, or null. Overlays belong to it, so the
   *  builder asks for sign-in until there is one. */
  accountId: string | null;
  /** Start Twitch sign-in. */
  signIn(): void;
  /** A call to the overlays API (`/api/overlays`, `/api/lookup/...`). */
  request(method: 'GET' | 'POST' | 'DELETE', path: string, query?: string, body?: unknown): Promise<OverlayApiResponse>;
  /** The identifier to store for a parsed YouTube identifier (a legacy /c/ or
   *  /user/ link becomes its UC id). Rejects with a line for the add box. */
  resolveYouTubeIdentifier(id: string): Promise<string>;
  /** The Kick slug to store for a name that has two spellings. */
  resolveKickSlug(slug: string): Promise<string>;
  /** A YouTube channel's name for its UC id, or null. */
  youTubeChannelTitle(channelId: string): Promise<string | null>;
  /** A lookup's rejection as a line for the add box. */
  lookupError(err: unknown, platform: string): string;
  /** The live-chat preview feed. `overlayId` is the published overlay being
   *  edited, when there is one (the site's relay feed is keyed by it). */
  LivePreview: ComponentType<{
    sources: OverlaySourceRef[];
    style: OverlayStyle;
    superSample?: number;
    overlayId?: string | null;
  }>;
}

const OverlayHostContext = createContext<OverlayHost | null>(null);

export const OverlayHostProvider = OverlayHostContext.Provider;

export function useOverlayHost(): OverlayHost {
  const host = useContext(OverlayHostContext);
  if (!host) throw new Error('The overlay builder needs an OverlayHostProvider.');
  return host;
}
