import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { MultiNookSlot } from '../types';

// The stores reach the backend and the app store at import time. Neither is
// under test here: this file is about slot identity, which is pure.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('./AppStore', () => {
  const state = { addToast: vi.fn(), settings: {}, currentStream: null, updateSettings: vi.fn() };
  return { useAppStore: Object.assign(vi.fn(), { getState: () => state, setState: vi.fn(), subscribe: vi.fn() }) };
});

import { activeChatSlot, usemultiNookStore } from './multiNookStore';
import { slotToPresetChannel } from './multiNookPresetsStore';

function slot(over: Partial<MultiNookSlot> & { channelLogin: string }): MultiNookSlot {
  return { id: `id-${over.channelLogin}-${over.provider ?? 'twitch'}`, volume: 1, muted: false, isFocused: false, ...over };
}

// Before this migration a slot was identified by a bare lowercased login, so a
// Kick channel and a Twitch channel of the same name were ONE slot: adding the
// second was refused as a duplicate, and removing either removed both.
describe('slot identity is provider-composite', () => {
  const twitchXqc = slot({ channelLogin: 'xqc' });
  const kickXqc = slot({ channelLogin: 'xqc', provider: 'kick' });

  it('treats the same login on two platforms as two distinct slots', () => {
    const slots = [twitchXqc, kickXqc];
    expect(activeChatSlot(slots, 'twitch:xqc')).toBe(twitchXqc);
    expect(activeChatSlot(slots, 'kick:xqc')).toBe(kickXqc);
  });

  // Legacy grids and any Twitch-era caller still address tiles by bare login.
  it('resolves a legacy bare login to the Twitch slot', () => {
    expect(activeChatSlot([twitchXqc, kickXqc], 'xqc')).toBe(twitchXqc);
  });

  it('returns null for no active id and for an id no slot owns', () => {
    expect(activeChatSlot([twitchXqc], null)).toBeNull();
    expect(activeChatSlot([twitchXqc], 'kick:xqc')).toBeNull();
  });
});

// The serde trap: multi_nook_slots is a TYPED field on the Rust Settings struct,
// so a field the struct does not name is dropped on the first save. The Rust
// side has its own round-trip test; this asserts the TS side actually emits it.
describe('slotToPresetChannel carries provider into the persisted shape', () => {
  it('keeps a non-Twitch provider', () => {
    expect(slotToPresetChannel(slot({ channelLogin: 'somebody', provider: 'kick' })).provider).toBe('kick');
  });

  it('leaves provider absent for Twitch rather than writing it out', () => {
    expect(slotToPresetChannel(slot({ channelLogin: 'xqc' })).provider).toBeUndefined();
  });

  it('does not persist transient view state', () => {
    const persisted = slotToPresetChannel(slot({ channelLogin: 'xqc', isFocused: true, isMinimized: true, title: 'live now' }));
    for (const k of ['volume', 'muted', 'isFocused', 'isMinimized', 'title', 'streamUrl', 'loadError']) {
      expect(persisted).not.toHaveProperty(k);
    }
  });
});

// The gate. Phase 6 opened it for Kick; YouTube followed once its relay was
// keyed by stream id rather than living in process-wide statics. The rule lives
// in one predicate (canGridProvider) so the surfaces cannot drift, and these
// cases exist mostly to catch a provider being admitted by accident.
describe('addSlot provider gate', () => {
  beforeEach(() => { usemultiNookStore.setState({ slots: [] }); });

  it('ADMITS Kick', async () => {
    await usemultiNookStore.getState().addSlot('somebody', 'kick');
    const slots = usemultiNookStore.getState().slots;
    expect(slots).toHaveLength(1);
    expect(slots[0].provider).toBe('kick');
  });

  // The mocked provider_channel_meta resolves null, which is the real
  // unresolvable-channel case too: the tile must still be usable, named after
  // its login, rather than throwing or rendering blank.
  it('falls back to the login when the platform returns no metadata', async () => {
    await usemultiNookStore.getState().addSlot('somebody', 'kick');
    expect(usemultiNookStore.getState().slots[0].channelName).toBe('somebody');
  });

  it('admits YouTube now that its relay is keyed per stream', async () => {
    await usemultiNookStore.getState().addSlot('AGr94tpNVkw', 'youtube');
    const slots = usemultiNookStore.getState().slots;
    expect(slots).toHaveLength(1);
    expect(slots[0].provider).toBe('youtube');
  });

  it('lets the same login exist once per platform', async () => {
    await usemultiNookStore.getState().addSlot('xqc', 'twitch');
    await usemultiNookStore.getState().addSlot('xqc', 'kick');
    expect(usemultiNookStore.getState().slots).toHaveLength(2);
  });

  it('still refuses a true duplicate on the same platform', async () => {
    await usemultiNookStore.getState().addSlot('xqc', 'kick');
    await usemultiNookStore.getState().addSlot('xqc', 'kick');
    expect(usemultiNookStore.getState().slots).toHaveLength(1);
  });

  it('stores Twitch as an absent provider, not an explicit one', async () => {
    await usemultiNookStore.getState().addSlot('xqc', 'twitch');
    expect(usemultiNookStore.getState().slots[0].provider).toBeUndefined();
  });
});

// The gate is only as good as its narrowest path. loadPresetChannels builds
// slots INLINE in replace mode and never calls addSlot, so before Phase 6 it was
// a way around the refusal entirely. Both of its modes must enforce the same rule.
describe('preset loading enforces the same gate', () => {
  const channels = [
    { channelLogin: 'ontwitch' },
    { channelLogin: 'onkick', provider: 'kick' as const },
    { channelLogin: 'AGr94tpNVkw', provider: 'youtube' as const },
  ];

  beforeEach(() => { usemultiNookStore.setState({ slots: [] }); });

  it('builds every eligible channel in replace mode, which never calls addSlot', async () => {
    await usemultiNookStore.getState().loadPresetChannels(channels, 'replace');
    const slots = usemultiNookStore.getState().slots;
    expect(slots.map((s) => s.channelLogin).sort()).toEqual(['AGr94tpNVkw', 'onkick', 'ontwitch']);
    expect(slots.find((s) => s.channelLogin === 'onkick')?.provider).toBe('kick');
  });

  it('agrees with replace mode, so the two buttons behave the same', async () => {
    await usemultiNookStore.getState().loadPresetChannels(channels, 'append');
    expect(usemultiNookStore.getState().slots.map((s) => s.channelLogin).sort())
      .toEqual(['AGr94tpNVkw', 'onkick', 'ontwitch']);
  });

  it('round-trips a Kick channel through the preset shape and back to a slot', async () => {
    const persisted = slotToPresetChannel(slot({ channelLogin: 'somebody', provider: 'kick' }));
    await usemultiNookStore.getState().loadPresetChannels([persisted], 'replace');
    expect(usemultiNookStore.getState().slots[0].provider).toBe('kick');
  });

  it('normalizes an explicit twitch provider to absent, matching addSlot', async () => {
    await usemultiNookStore.getState().loadPresetChannels(
      [{ channelLogin: 'xqc', provider: 'twitch' as const }],
      'replace',
    );
    expect(usemultiNookStore.getState().slots[0].provider).toBeUndefined();
  });
});

// Removal has to agree with the gate's key space, or recalling one platform's
// ghost card removes the other platform's tile.
describe('removeSlotByLogin is provider-aware', () => {
  it('removes only the named platform', async () => {
    usemultiNookStore.setState({
      slots: [
        slot({ id: 'cell-t', channelLogin: 'xqc' }),
        slot({ id: 'cell-k', channelLogin: 'xqc', provider: 'kick' }),
      ],
    });
    await usemultiNookStore.getState().removeSlotByLogin('xqc', 'kick');
    const left = usemultiNookStore.getState().slots;
    expect(left).toHaveLength(1);
    expect(left[0].id).toBe('cell-t');
  });
});

// REGRESSION. Phase 3 moved the chat selection to a composite key but left five
// writers setting a raw numeric channelId or a bare login. Every write LOOKED
// fine (the field is a string, nothing threw) while no consumer could match it,
// so on Twitch focusing a tile, maximizing one, or restoring one from the tray
// silently deselected chat. The invariant these tests hold is narrow and total:
// whatever writes activeChatChannelId must write something activeChatSlot can
// resolve back to that same slot.
describe('activeChatChannelId always lands in the key space its readers compare', () => {
  const store = () => usemultiNookStore.getState();
  const slots = [
    slot({ id: 'cell-a', channelLogin: 'alpha', channelId: '11111' }),
    slot({ id: 'cell-b', channelLogin: 'beta', channelId: '22222' }),
  ];

  beforeEach(() => {
    usemultiNookStore.setState({ slots: slots.map((s) => ({ ...s })), activeChatChannelId: null, maximizedSlotId: null });
  });

  function resolvesToSlotId(expectedId: string) {
    const { slots: now, activeChatChannelId } = store();
    expect(activeChatChannelId).not.toBeNull();
    // The real assertion: not the key's spelling, but that a reader gets the slot.
    expect(activeChatSlot(now, activeChatChannelId)?.id).toBe(expectedId);
  }

  it('resolves after focusing a tile', () => {
    store().toggleFocusSlot('cell-b');
    resolvesToSlotId('cell-b');
  });

  it('resolves after maximizing a tile', () => {
    store().toggleMaximizeSlot('cell-b');
    resolvesToSlotId('cell-b');
  });

  // The numeric channelId is the exact value the old code wrote. Naming it here
  // means a revert to id-keying fails loudly instead of going quiet again.
  it('never writes a raw channelId', () => {
    store().toggleFocusSlot('cell-b');
    expect(store().activeChatChannelId).not.toBe('22222');
  });

  // The switcher writes this field directly, so its key must round-trip too.
  it('resolves a key written straight through the setter', () => {
    store().setActiveChatChannelId('twitch:alpha');
    resolvesToSlotId('cell-a');
  });
});
