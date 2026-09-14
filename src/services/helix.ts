// Helix reads, made by Rust. The page names the resource and the query; Rust
// attaches the client id and the OAuth token (src-tauri/src/commands/helix.rs,
// allowlisted resources) and returns the JSON. Nothing here ever sees a token.
// A non-2xx answer rejects with `helix_<status>: <body>`, so a caller that used
// to branch on `resp.ok` branches on the catch instead.
import { invoke } from '@tauri-apps/api/core';

export type HelixResource = 'users' | 'streams' | 'channels' | 'clips';

export function helixGet<T = unknown>(resource: HelixResource, query: string): Promise<T> {
  return invoke<T>('helix_get', { resource, query });
}

/** `key=value&key=value` from a list, encoded, for the batch endpoints. */
export function helixQuery(key: string, values: readonly string[]): string {
  return values.map((v) => `${key}=${encodeURIComponent(v)}`).join('&');
}
