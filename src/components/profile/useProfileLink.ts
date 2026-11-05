// Asks Rust for a member's StreamNook page link and says how the plate should
// draw (desktop: the member hero and Settings > Profile). The question is asked
// again when the member list changes (a late-loading registry turns "not ready"
// into a link) and when the login arrives.
import { useEffect, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getStreamNookRegistryVersion,
  subscribeStreamNookRegistryVersion,
} from '../../services/supabaseService';
import type { PageLink, PageLinkAnswer } from '../../types/profileView';
import { desktopPlateSlot, profileLinkAsk, profileLinkKey, type PlateSlot } from './profileLink';

export function useProfileLink(opts: {
  memberId: string | null | undefined;
  login: string | null | undefined;
  loadFinished: boolean;
  expectMember: boolean;
}): { slot: PlateSlot; link: PageLink | null } {
  const { memberId, login, loadFinished, expectMember } = opts;
  const registryVersion = useSyncExternalStore(
    subscribeStreamNookRegistryVersion,
    getStreamNookRegistryVersion,
    getStreamNookRegistryVersion,
  );
  const [answered, setAnswered] = useState<{ key: string; value: PageLinkAnswer } | null>(null);

  useEffect(() => {
    const ask = profileLinkAsk(memberId, login);
    if (!ask) return;
    const key = `${ask.memberId}|${ask.login}`;
    let cancelled = false;
    invoke<PageLinkAnswer>('streamnook_profile_link', { memberId: ask.memberId, login: ask.login })
      .then((value) => {
        if (!cancelled) setAnswered({ key, value });
      })
      .catch(() => {
        // Nothing to draw on a failed ask; the next registry change asks again.
        if (!cancelled) setAnswered({ key, value: { state: 'not_ready' } });
      });
    return () => {
      cancelled = true;
    };
  }, [memberId, login, registryVersion]);

  const key = profileLinkKey(memberId, login);
  const answer = key !== null && answered?.key === key ? answered.value : null;
  return {
    slot: desktopPlateSlot({ answer, asked: key !== null, loadFinished, expectMember }),
    link: answer?.link ?? null,
  };
}
