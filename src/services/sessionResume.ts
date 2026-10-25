import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { Logger } from '../utils/logger';
import { makeKey, parseKey } from '../utils/providerKey';
import { markOwnChannelOpened, notePauseIntent, pauseIntentOf } from '../utils/pausePark';
import { streamKey, streamProvider } from '../utils/streamProvider';

/** Mirrors the Rust `ResumeSnapshot` struct (commands/session.rs). */
interface ResumeSnapshot {
  stream_login?: string | null;
  media_type?: string | null;
  original_media_url?: string | null;
  was_running: boolean;
  automation_campaign_id?: string | null;
  paused?: boolean;
  multinook_active?: boolean;
}

/**
 * Snapshot what the user is doing so the next launch can restore it. Called
 * right before the app restarts (an update, or a restart after the GPU
 * process came back without the GPU). Captures a live stream (clips and VODs
 * are skipped), whether the viewer had paused it, and whether MultiNook is open.
 */
export async function captureResumeSnapshot(): Promise<void> {
  const s = useAppStore.getState();
  const stream = s.currentStream;
  const live = stream?.user_login && s.currentMediaType === 'live' ? stream : null;
  const multinookActive = usemultiNookStore.getState().isMultiNookActive;
  if (!live && !multinookActive) return;

  // The COMPOSITE key, so a Kick stream resumes on Kick. Twitch rows still key
  // as a bare login, so snapshots written before multi-platform browsing (and
  // every Twitch snapshot after it) restore exactly as they always did.
  const isTwitch = !!live && streamProvider(live) === 'twitch';
  const intent = live ? pauseIntentOf('main') : null;
  const snapshot: ResumeSnapshot = {
    stream_login: live ? streamKey(live) : null,
    media_type: live ? s.currentMediaType : null,
    original_media_url: live ? (s.originalMediaUrl ?? null) : null,
    // Drops and their automation are Twitch-only, so a provider snapshot must
    // not ask the next launch to replay them.
    was_running: isTwitch && s.dropProgressActive,
    automation_campaign_id: isTwitch ? (s.liveDropProgress?.current_campaign ?? null) : null,
    // The viewer's live pause, keyed as the player keys it.
    paused: !!live && !!intent && intent.key === makeKey(streamProvider(live), live.user_login),
    multinook_active: multinookActive,
  };

  try {
    await invoke('save_resume_snapshot', { snapshot });
  } catch (e) {
    Logger.error('[resume] Failed to save snapshot:', e);
  }
}

/**
 * Consume the resume snapshot (if any) and put the user back. Reopening the
 * stream via startStream also re-arms chat and the built-in watched-channel
 * drops monitor, so only the opt-in plugin automation needs an explicit replay.
 * A paused stream is held before it opens, so its player never starts playing;
 * MultiNook reopens with the tiles saved in settings.
 * Best-effort throughout: a failure here must never block app boot.
 */
export async function resumePreviousSession(): Promise<void> {
  let snap: ResumeSnapshot | null = null;
  try {
    snap = await invoke<ResumeSnapshot | null>('take_resume_snapshot');
  } catch (e) {
    Logger.error('[resume] Failed to read snapshot:', e);
    return;
  }
  if (!snap) return;

  if (snap.stream_login) {
    const { provider, channel } = parseKey(snap.stream_login);
    if (snap.paused) {
      notePauseIntent('main', makeKey(provider, channel));
      Logger.info('[resume] the stream was paused; it reopens paused');
    } else {
      // It was playing, so the viewer already opened it: if it is their own
      // channel it must not open paused a second time.
      markOwnChannelOpened(makeKey(provider, channel));
    }
    Logger.info(`[resume] Restoring stream: ${snap.stream_login}`);
    let reopened = true;
    try {
      await useAppStore.getState().startStream(snap.stream_login);
    } catch (e) {
      reopened = false;
      Logger.error('[resume] Failed to reopen stream:', e);
    }
    if (reopened && snap.was_running) {
      void replayAutomation(snap.automation_campaign_id ?? null);
    }
  }

  if (snap.multinook_active && !usemultiNookStore.getState().isMultiNookActive) {
    Logger.info('[resume] Reopening MultiNook');
    void usemultiNookStore.getState().toggleMultiNook();
  }
}

/**
 * Re-arm the opt-in plugin automation. The plugin host launches enabled plugins on
 * startup, but the drops-automation provider may not be registered the instant we
 * get here, so poll briefly for it before replaying the action.
 */
async function replayAutomation(campaignId: string | null): Promise<void> {
  for (let attempt = 0; attempt < 10; attempt++) {
    try {
      const providerId = await invoke<string | null>('plugins_provides', {
        feature: 'drops.automation',
      });
      if (providerId) {
        if (campaignId) {
          await invoke('plugins_invoke_action', {
            action: 'drops.run',
            args: { campaign_id: campaignId },
          });
        } else {
          await invoke('plugins_invoke_action', { action: 'drops.run-auto', args: {} });
        }
        Logger.info('[resume] Automation resumed.');
        return;
      }
    } catch (e) {
      Logger.error('[resume] Automation replay attempt failed:', e);
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  Logger.warn('[resume] No drops.automation provider became available; skipped automation resume.');
}
