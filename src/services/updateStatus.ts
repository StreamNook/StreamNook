/**
 * Running an update check on demand, and saying what happened in plain words.
 *
 * WHY THIS EXISTS
 * On 2026-09-19, 114 of 181 recently-active members were not on the latest
 * release, and a user who suspected they were behind had NO way to act on that:
 * there was no "check for updates" control anywhere in the app, the About dialog
 * showed a bare version string with no comparison, and every automatic check
 * swallowed its failures (`Logger.warn`, never surfaced). The only signal that a
 * new build existed was a 26px title-bar pill, which needs the window open and
 * looked at, and which looks identical whether you are one patch or twelve
 * releases behind.
 *
 * This module is the on-demand check the changelog popup runs. The periodic check
 * in `DynamicIsland` owns the notification bookkeeping and keeps its own copy of
 * the invoke.
 */

import { invoke } from '@tauri-apps/api/core';

/** The fields of Rust's `BundleUpdateStatus` this module uses. */
export interface BundleUpdateStatus {
  update_available: boolean;
  current_version: string;
  latest_version: string;
  download_size?: string | null;
  /** Published releases between the two, counted in Rust from its cached
   *  release list. Null when that list is cold. */
  releases_behind?: number | null;
}

export type UpdateCheckResult =
  | { kind: 'current'; version: string }
  | { kind: 'available'; status: BundleUpdateStatus; behind: number | null }
  | { kind: 'failed'; reason: string };

/**
 * Run a check now.
 *
 * Never throws: a failure is a result the caller shows, not an exception it
 * swallows. That is the whole difference from the automatic path, whose errors
 * went to `Logger.warn` and left the user looking at nothing.
 */
export async function checkNow(): Promise<UpdateCheckResult> {
  try {
    const status = await invoke<BundleUpdateStatus>('check_for_bundle_update');
    if (!status.update_available) {
      return { kind: 'current', version: status.current_version };
    }
    return {
      kind: 'available',
      status,
      behind: status.releases_behind ?? null,
    };
  } catch (e) {
    return { kind: 'failed', reason: describeFailure(e) };
  }
}

/**
 * Turn a Rust error string into something a person can act on.
 *
 * The raw strings are shaped like "Failed to fetch update manifest: error
 * sending request for url (...)", which tells a user nothing they can do.
 */
function describeFailure(e: unknown): string {
  const raw = String((e as { message?: string })?.message ?? e ?? '');
  if (/timed? ?out|timeout/i.test(raw)) {
    return 'The update server did not answer in time. Check your connection and try again.';
  }
  if (/dns|resolve|lookup/i.test(raw)) {
    return 'Could not look up streamnook.app. That is usually a DNS or VPN filter on this network.';
  }
  if (/Failed to fetch update manifest|error sending request|connect/i.test(raw)) {
    return 'Could not reach the update server. Check your connection, VPN or firewall.';
  }
  if (/returned an error|HTTP \d/i.test(raw)) {
    return 'The update server answered with an error. This is on our side; try again shortly.';
  }
  if (/parse/i.test(raw)) {
    return 'The update information could not be read. This is on our side; try again shortly.';
  }
  return raw || 'The update check failed for an unknown reason.';
}
