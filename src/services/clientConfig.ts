/**
 * Server-controlled client switches, read from the update manifest by Rust
 * (services/client_config.rs), which caches them for every window and fails
 * open to the legacy behaviour when the manifest cannot be reached.
 */

import { invoke } from '@tauri-apps/api/core';

export interface ClientConfig {
    /** Route privileged writes through the StreamNook API instead of Supabase. */
    write_via_api: boolean;
    /** Builds below this are asked to update; empty means no floor. */
    min_supported_version: string;
}

const LEGACY: ClientConfig = { write_via_api: false, min_supported_version: '' };

export const getClientConfig = (): Promise<ClientConfig> =>
    invoke<ClientConfig>('get_client_config').catch(() => LEGACY);

/** Warm the cache at startup so the first write does not pay the fetch. */
export const primeClientConfig = (): void => {
    void getClientConfig();
};
