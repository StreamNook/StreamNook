// Generic watch-to-earn reward claims. While a user watches a stream, any active
// EVENT reward in Supabase whose eligibility matches what they're watching is
// claimed. The server decides eligibility per reward (its window + category list
// + channel allowlist) and records every attempt, so a new watch event is pure
// config (a rewards row) with no client release. Called from the watch-time
// tracker (App.tsx) on stream start, once a minute while watching, and the moment
// the watched channel's category changes.
import { invoke } from '@tauri-apps/api/core';
import { claimReward, listActiveEventRewards, listActiveMilestoneRewards, refreshEntitlementRegistries } from './supabaseService';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';

// Stop re-claiming a reward once the server gives a definitive answer this
// session (key `${userId}:${rewardId}`), and latch a reward whose window the
// server reports closed (key `rewardId`) so we stop asking for it.
const grantedThisSession = new Set<string>();
const windowClosed = new Set<string>();

export async function maybeClaimWatchRewards(
  userId: string | undefined | null,
  channelLogin: string | undefined | null,
  gameNameHint: string | null | undefined,
): Promise<void> {
  if (!userId || !channelLogin) return;

  const rewards = await listActiveEventRewards();
  if (!rewards.length) return;

  // The stream object's game_name is empty on some open paths (direct channel
  // open, raids); fetch the live category in that case. A mid-stream switch is
  // kept fresh by the EventSub channel.update that updates currentStream.
  let game = gameNameHint ?? null;
  if (!game) {
    try {
      const info = await invoke<{ game_name?: string }>('get_channel_info', { channelName: channelLogin });
      game = info?.game_name ?? game;
    } catch (e) {
      Logger.warn('[WatchRewards] get_channel_info failed:', e);
    }
  }

  // The server can't verify what was watched, so we report it as context and let
  // the reward's eligibility decide.
  const context = { category: game ?? '', channel: channelLogin };
  await claimEventRewards(userId, rewards, context);
}

// Login-window rewards (e.g. a holiday event with no category/channel rules):
// claimed once at sign-in with empty context. Rewards that DO carry watch
// eligibility just answer not_eligible here, so this stays generic.
export async function maybeClaimLoginRewards(userId: string | undefined | null): Promise<void> {
  if (!userId) return;
  const rewards = await listActiveEventRewards();
  if (!rewards.length) return;
  await claimEventRewards(userId, rewards, {});
}

// Milestone rewards (subscriber-tenure badges gated on total_months): claimed
// once at sign-in. The server checks the real stored month count, so a member
// at/over the threshold is granted and everyone else just answers
// below_threshold. New tiers are pure config (a rewards row), no client release.
export async function maybeClaimMilestoneRewards(userId: string | undefined | null): Promise<void> {
  if (!userId) return;
  const rewards = await listActiveMilestoneRewards();
  if (!rewards.length) return;
  await claimEventRewards(userId, rewards, {});
}

async function claimEventRewards(
  userId: string,
  rewards: Awaited<ReturnType<typeof listActiveEventRewards>>,
  context: Record<string, string>,
): Promise<void> {
  for (const reward of rewards) {
    if (windowClosed.has(reward.id)) continue;
    const sessionKey = `${userId}:${reward.id}`;
    if (grantedThisSession.has(sessionKey)) continue;

    const res = await claimReward(userId, reward.id, context);

    if (res.reason === 'window_closed') {
      windowClosed.add(reward.id);
      continue;
    }
    if (!res.ok) {
      // not_eligible (event mismatch) and below_threshold (milestone not yet
      // reached) are the normal "not for you right now" answers; only the
      // unexpected failures (RLS / network / missing fn) warn.
      if (res.reason !== 'not_eligible' && res.reason !== 'below_threshold') {
        Logger.warn(`[WatchRewards] claim failed reward=${reward.id} context=${JSON.stringify(context)}: ${res.reason ?? 'unknown'}`);
      }
      continue;
    }

    // Definitive answer this session; stop re-claiming this reward for this user.
    grantedThisSession.add(sessionKey);

    if (res.granted) {
      // This claim just wrote the grant, so re-pull now rather than wait for a
      // realtime event: at sign-in the channel may not be subscribed yet.
      refreshEntitlementRegistries();
      // alwaysShow so a genuinely new earn lands even when routine toasts are
      // muted. Granting only UNLOCKS the look; the member applies it themselves.
      useAppStore.getState().addToast(
        `Achievement unlocked: ${reward.title ?? 'event reward'}. Apply it from profile customization.`,
        'success',
        undefined,
        { alwaysShow: true },
      );
      Logger.info(`[WatchRewards] granted reward=${reward.id} to ${userId}`);
    }
  }
}
