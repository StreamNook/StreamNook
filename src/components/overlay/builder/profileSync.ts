// How the builder's overlay list lines up with the account's overlays.
//
// Published overlays belong to the Twitch account and live on the server, so the
// server copy is the truth for each of them. Drafts (never published) exist only
// in this app or browser and are left exactly as they are. Pure, so both the app
// and the site run the same rule and it can be tested without either.

import { clampOverlayStyle, DEFAULT_OVERLAY_STYLE, type OverlayStyle } from '../overlayConfig';
import { PROVIDERS, type ProviderId } from '../../../types/providers';

export interface OverlaySource {
  provider: ProviderId;
  channel: string;
}

export interface OverlayProfile {
  /** Stable client-side identity. Save responses are routed by uid, so a
   *  mid-flight switch can never stamp one overlay's id onto another. Never sent. */
  uid: string;
  name: string;
  /** The published overlay's id (its OBS link), or null for a draft. */
  id: string | null;
  /** The server version this copy was last saved at or loaded from. Sent with
   *  each save so a newer save from elsewhere is never overwritten. */
  version: string | null;
  style: OverlayStyle;
  sources: OverlaySource[];
}

/** One overlay as the account list returns it. */
export interface ServerOverlay {
  id?: unknown;
  channels?: unknown;
  style?: unknown;
  updated_at?: unknown;
}

export const newProfileUid = (): string =>
  typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `p-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

/** A server row as a profile, or null when it is malformed. `uid` lets a local
 *  copy keep its identity when it is refreshed from the server. */
export function profileFromServer(row: ServerOverlay, fallbackName: string, uid = newProfileUid()): OverlayProfile | null {
  if (typeof row.id !== 'string' || !row.id) return null;
  const st = (row.style && typeof row.style === 'object' ? row.style : {}) as Record<string, unknown>;
  const sources = Array.isArray(row.channels)
    ? (row.channels as Array<{ provider?: unknown; channel?: unknown }>)
        .filter((c) => typeof c?.channel === 'string' && !!PROVIDERS[c.provider as ProviderId])
        .map((c) => ({ provider: c.provider as ProviderId, channel: c.channel as string }))
    : [];
  return {
    uid,
    name: typeof st.profileName === 'string' && st.profileName.trim() ? st.profileName : fallbackName,
    id: row.id,
    version: typeof row.updated_at === 'string' ? row.updated_at : null,
    style: clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...st } as OverlayStyle),
    sources,
  };
}

export interface Reconciled {
  profiles: OverlayProfile[];
  /** Names of local overlays that are no longer on the account (deleted on
   *  another device, or they belong to a different Twitch account). */
  dropped: string[];
}

/**
 * The builder's list after hearing the account's overlays.
 *
 * - A local overlay the server has takes the server copy (keeping its uid).
 * - A local overlay the server does not have is dropped: it was deleted
 *   elsewhere, or it belongs to another account. Either way its link is not
 *   this account's to edit.
 * - Server overlays not in the list are added after the local ones.
 * - Drafts stay as they are.
 * - The list is never empty: a lone default draft stands in.
 */
export function reconcileProfiles(local: OverlayProfile[], server: ServerOverlay[]): Reconciled {
  // The blank starting overlay a first open creates (one draft, no channels) is
  // not somebody's draft; it only holds the page until the account's overlays
  // arrive, so it gives way to them.
  const blankStart = local.length === 1 && !local[0].id && local[0].sources.length === 0;
  if (blankStart && server.some((r) => typeof r.id === 'string' && r.id)) local = [];

  const byId = new Map<string, ServerOverlay>();
  for (const row of server) if (typeof row.id === 'string' && row.id) byId.set(row.id, row);

  const out: OverlayProfile[] = [];
  const dropped: string[] = [];
  const seen = new Set<string>();
  for (const p of local) {
    if (!p.id) {
      out.push(p);
      continue;
    }
    const row = byId.get(p.id);
    if (!row || seen.has(p.id)) {
      if (!row) dropped.push(p.name);
      continue;
    }
    seen.add(p.id);
    out.push(profileFromServer(row, p.name, p.uid) ?? p);
  }
  let n = out.length;
  for (const row of server) {
    if (typeof row.id !== 'string' || seen.has(row.id)) continue;
    const p = profileFromServer(row, `Overlay ${++n}`);
    if (p) {
      seen.add(p.id as string);
      out.push(p);
    }
  }
  if (out.length === 0) {
    out.push({ uid: newProfileUid(), name: 'Default', id: null, version: null, style: { ...DEFAULT_OVERLAY_STYLE }, sources: [] });
  }
  return { profiles: out, dropped };
}
