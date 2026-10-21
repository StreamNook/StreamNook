// What a stream's category label opens, and how Home gets there.
//
// TypeScript on purpose: the answer is Home's navigation state, which lives in
// the app store, and the inputs are three fields already on the row plus the
// platform table in types/providers. A Rust call would only add a round trip
// per render. Rust keeps the id itself right: the watch session sends it with
// every category change.
//
// Twitch opens by id when the row has one and by name when it does not.
// Another platform opens only where it has a category directory and the row
// carries that platform's own id: a Kick id means nothing to Twitch's lookup,
// and a name alone cannot address a Kick category.

import type { TwitchCategory, TwitchStream } from '../types';
// Value imports carry the .ts extension, matching the other tested utils.
import { PROVIDER_WATCH, type ProviderCategory, type ProviderId } from '../types/providers.ts';
import { streamProvider } from './streamProvider.ts';

export type StreamCategoryTarget =
  | { kind: 'twitch'; category: TwitchCategory }
  | { kind: 'provider'; category: ProviderCategory };

type Row = Pick<TwitchStream, 'provider' | 'game_id' | 'game_name'>;

export function streamCategoryTarget(stream: Row | null | undefined): StreamCategoryTarget | null {
  const name = stream?.game_name?.trim();
  if (!stream || !name) return null;
  const provider = streamProvider(stream);
  const id = stream.game_id?.trim() || '';
  if (provider === 'twitch') {
    return { kind: 'twitch', category: { id, name, box_art_url: '' } };
  }
  if (PROVIDER_WATCH[provider]?.browse !== 'categories' || !id) return null;
  return {
    kind: 'provider',
    category: { provider, id, name, thumbnail: '', viewer_count: 0, channel_count: 0 },
  };
}

/** The target for what the player shows. Clips and VODs carry a placeholder
 *  name ("Twitch Clip"), so they open nothing; the offline room's broadcast
 *  keeps the channel's live id beside the VOD's own name, so it goes by name. */
export function currentCategoryTarget(
  stream: Row | null | undefined,
  mediaType: 'live' | 'clip' | 'video' | 'offline_chat' | null,
): StreamCategoryTarget | null {
  if (!stream) return null;
  if (mediaType === 'live') return streamCategoryTarget(stream);
  if (mediaType === 'offline_chat') return streamCategoryTarget({ ...stream, game_id: undefined });
  return null;
}

export type CategoryNavigation =
  | {
      platform: 'twitch' | null;
      patch: {
        homeActiveTab: 'category';
        homeSelectedCategory: TwitchCategory;
        homeLastExitedCategory: null;
        homeCategoryTab: 'live';
      };
    }
  | {
      platform: ProviderId | null;
      patch: {
        homeActiveTab: 'browse';
        homeProviderCategory: ProviderCategory;
        homeLastExitedProviderCategory: null;
      };
    };

/** How the store opens a target: the platform to switch to first (null when
 *  the current one already shows it), then the state to set. Opening is a new
 *  destination, so the forward step goes, and a category always opens on Live. */
export function categoryNavigation(
  target: StreamCategoryTarget,
  activePlatform: ProviderId | 'all',
): CategoryNavigation {
  if (target.kind === 'twitch') {
    return {
      platform: activePlatform === 'all' || activePlatform === 'twitch' ? null : 'twitch',
      patch: {
        homeActiveTab: 'category',
        homeSelectedCategory: target.category,
        homeLastExitedCategory: null,
        homeCategoryTab: 'live',
      },
    };
  }
  return {
    platform: activePlatform === target.category.provider ? null : target.category.provider,
    patch: {
      homeActiveTab: 'browse',
      homeProviderCategory: target.category,
      homeLastExitedProviderCategory: null,
    },
  };
}
