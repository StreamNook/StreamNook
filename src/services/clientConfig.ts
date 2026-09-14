/**
 * Server-controlled client switches, read from the update manifest.
 *
 * Exists so the writes that move from direct-Supabase to the StreamNook API can
 * be turned back off WITHOUT shipping a build. A desktop release cannot be
 * recalled: if the new write path has a bug, is rate-limited, or an endpoint is
 * rolled back, every user on that build silently loses writes until they install
 * another one. Flipping one field in `latest.json` reverts them in minutes.
 *
 * Rides on the update manifest rather than a new endpoint because that file is
 * already fetched, already edge-cached (~300s), and already the thing Brandon
 * re-uploads to ship. One less surface to keep alive.
 *
 * FAILS OPEN, deliberately. If the config cannot be fetched we return the legacy
 * behaviour (direct Supabase), because the alternative is that a Cloudflare blip
 * takes writes down as well. The legacy path stays functional until the anon
 * write policies are revoked, which is a later, separately-gated step.
 */

import { Logger } from '../utils/logger';

const MANIFEST_URL = 'https://streamnook.app/api/v1/update';
const TTL_MS = 5 * 60 * 1000;

export interface ClientConfig {
    /** Route privileged writes through the StreamNook API instead of Supabase. */
    writeViaApi: boolean;
    /** Builds below this are asked to update; empty means no floor. */
    minSupportedVersion: string;
}

// Legacy behaviour. Also what a failed fetch resolves to.
const FALLBACK: ClientConfig = { writeViaApi: false, minSupportedVersion: '' };

let cached: ClientConfig | null = null;
let cachedAt = 0;
let inFlight: Promise<ClientConfig> | null = null;

async function fetchConfig(): Promise<ClientConfig> {
    try {
        const res = await fetch(MANIFEST_URL, { cache: 'no-store' });
        if (!res.ok) return FALLBACK;
        const body = (await res.json()) as { client_config?: Record<string, unknown> };
        const cc = body.client_config;
        if (!cc || typeof cc !== 'object') return FALLBACK;
        return {
            // Only an explicit `true` enables it. A malformed or missing value
            // must never silently switch every client onto the new path.
            writeViaApi: cc.write_via_api === true,
            minSupportedVersion:
                typeof cc.min_supported_version === 'string' ? cc.min_supported_version : '',
        };
    } catch (e) {
        Logger.debug('[ClientConfig] fetch failed, using legacy behaviour:', e);
        return FALLBACK;
    }
}

/** Current config, cached for TTL_MS. Single-flight so a burst of writes at
 *  login does not fan out into N manifest fetches. */
export const getClientConfig = async (): Promise<ClientConfig> => {
    if (cached && Date.now() - cachedAt < TTL_MS) return cached;
    if (inFlight) return inFlight;
    inFlight = fetchConfig()
        .then((cfg) => {
            cached = cfg;
            cachedAt = Date.now();
            return cfg;
        })
        .finally(() => {
            inFlight = null;
        });
    return inFlight;
};

/** Sync read for hot paths that cannot await. Returns the legacy answer until
 *  the first successful fetch has landed, which is the safe direction. */
export const getClientConfigSync = (): ClientConfig => cached ?? FALLBACK;

/** Warm the cache at startup so the first write does not pay the fetch. */
export const primeClientConfig = (): void => {
    void getClientConfig();
};
