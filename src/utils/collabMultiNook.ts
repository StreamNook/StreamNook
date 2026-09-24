import type { Collaboration, MultiNookPresetChannel } from '../types';
import type { ProviderId } from '../types/providers';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { makeKey } from './providerKey';

// The grid itself is owned here in the frontend (multiNookStore; Rust only
// keeps the copy saved to settings), so comparing a group with it and handing
// the group over run against that store rather than a Rust command, which
// would read the saved copy and could lag an add still being saved. Who is in
// the group, and that each member is live, comes from Rust.

/** Members of a group not already tiles in the MultiNook grid. */
export function collabMissingFromGrid(
  collab: Collaboration,
  slots: { provider?: ProviderId; channelLogin: string }[],
): number {
  const inGrid = new Set(slots.map((s) => makeKey(s.provider ?? 'twitch', s.channelLogin)));
  return collab.members.filter((m) => !inGrid.has(makeKey('twitch', m.login))).length;
}

/**
 * Open every member of a Shared Viewership group in MultiNook.
 * `replace` makes the grid exactly the group (built from what the group
 * already carries, so it opens without a Twitch round trip per tile);
 * `append` adds the members the grid lacks through the normal add path, and
 * with none missing just brings the grid up.
 *
 * A solo stream that is one of the group hands over the way the player's own
 * "add to MultiNook" does: the grid comes up first and the solo player closes
 * keeping its backend, so its chat carries straight over. Any other solo
 * stream is closed outright first.
 */
export async function watchCollabInMultiNook(collab: Collaboration, mode: 'replace' | 'append'): Promise<void> {
  const { members } = collab;
  const channels: MultiNookPresetChannel[] = members.map((m) => ({
    channelLogin: m.login,
    channelId: m.user_id,
    channelName: m.display_name,
    profileImageUrl: m.avatar_url ?? undefined,
  }));

  const app = useAppStore.getState();
  const solo = app.streamUrl ? app.currentStream : null;
  const soloInGroup = !!solo && members.some((m) => m.login.toLowerCase() === solo.user_login?.toLowerCase());
  if (solo && !soloInGroup) await app.exitStream();

  const mn = usemultiNookStore.getState();
  if (mode === 'append') {
    // Only who the grid lacks, so the add path never reports a tile it has.
    const inGrid = new Set(mn.slots.map((s) => makeKey(s.provider ?? 'twitch', s.channelLogin)));
    const missing = channels.filter((c) => !inGrid.has(makeKey('twitch', c.channelLogin)));
    if (missing.length > 0) {
      await mn.setActivePresetId(null);
      await mn.loadPresetChannels(missing, 'append');
    }
  } else {
    await mn.loadPresetChannels(channels, 'replace');
  }

  // Chat follows the channel the viewer came from, else the group's first.
  const chatLogin = soloInGroup ? solo.user_login : members[0].login;
  usemultiNookStore.getState().setActiveChatChannelId(makeKey('twitch', chatLogin));

  if (!usemultiNookStore.getState().isMultiNookActive) {
    await usemultiNookStore.getState().toggleMultiNook();
  }
  if (soloInGroup) await useAppStore.getState().exitStream({ preserveBackend: true });
}
