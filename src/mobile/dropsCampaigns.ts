// Active campaigns by lowercase game name, shared across every card. Backend
// caches the campaign list (1h), so one invoke per session is plenty.
//
// A LIST per game, not one campaign. It used to keep a single entry, so the
// last campaign for a game overwrote the rest and whether the icon appeared
// depended on which one happened to arrive last. A game frequently has several
// campaigns running at once, some open to the whole category and some locked to
// a handful of channels, and the card has to consider all of them to answer
// "can this channel earn anything".
//
// Lives here rather than beside the stream card because the watch screen's drop
// progress bar asks the same question, and it must not import a card component
// to do it. `get_active_drop_campaigns` also forces a backend refetch, so having
// exactly one caller behind one cache matters more than tidiness.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { campaignEarnableOn } from './dropsEligibility';
import type { DropCampaign } from '../types';

export type DropsByGame = Map<string, DropCampaign[]>;

let dropsNamesCache: DropsByGame | null = null;
let dropsNamesPromise: Promise<DropsByGame> | null = null;

export function loadDropsGameNames(): Promise<DropsByGame> {
  if (dropsNamesCache) return Promise.resolve(dropsNamesCache);
  dropsNamesPromise ??= invoke<DropCampaign[]>('get_active_drop_campaigns')
    .then((campaigns) => {
      const map: DropsByGame = new Map();
      for (const campaign of campaigns ?? []) {
        if (!campaign.game_name) continue;
        const key = campaign.game_name.toLowerCase();
        const list = map.get(key);
        if (list) list.push(campaign);
        else map.set(key, [campaign]);
      }
      dropsNamesCache = map;
      return map;
    })
    .catch(() => new Map() as DropsByGame);
  return dropsNamesPromise;
}

export function useDropsGameNames(): DropsByGame {
  const [map, setMap] = useState<DropsByGame>(() => dropsNamesCache ?? new Map());
  useEffect(() => {
    if (!dropsNamesCache) void loadDropsGameNames().then(setMap);
  }, []);
  return map;
}

/**
 * Whether this channel, in this game, can earn anything at all right now.
 *
 * The same question the stream card's drops icon answers, asked by the watch
 * screen before it decides whether tracking progress is worth any work.
 */
export function channelHasEarnableCampaign(
  byGame: DropsByGame,
  gameName: string | null | undefined,
  channelLogin: string | null | undefined,
): boolean {
  if (!gameName) return false;
  return (byGame.get(gameName.toLowerCase()) ?? []).some((c) =>
    campaignEarnableOn(c, channelLogin),
  );
}
