// Web-rendered rows (desktop, Android) pick previewable links here; phone rows
// drawn natively get theirs from Rust (services/link_preview_cache.rs). Both must
// expand the same hosts, so the two allowlists are held equal.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { TRUSTED_BASE_DOMAINS, extractPreviewUrls, isTrustedHost } from './linkPreviewService';

const rustList = (): string[] => {
  const src = readFileSync(resolve(__dirname, '..', '..', 'src-tauri/src/services/link_preview_cache.rs'), 'utf8');
  const start = src.indexOf('pub const TRUSTED_BASE_DOMAINS');
  if (start < 0) throw new Error('TRUSTED_BASE_DOMAINS not found in link_preview_cache.rs');
  const block = src.slice(src.indexOf('[', src.indexOf('=', start)) + 1, src.indexOf('];', start));
  return [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
};

describe('link preview allowlist', () => {
  it('matches the Rust list the phone rows use', () => {
    expect([...TRUSTED_BASE_DOMAINS].sort()).toEqual(rustList().sort());
  });

  it('matches subdomains but not lookalikes', () => {
    expect(isTrustedHost('ptb.discord.com')).toBe(true);
    expect(isTrustedHost('notdiscord.gg')).toBe(false);
  });

  it('expands at most two trusted links, in order', () => {
    const urls = extractPreviewUrls('https://discord.gg/a https://youtu.be/x https://x.com/y').filter((u) => u.trusted);
    expect(urls.map((u) => u.url)).toEqual(['https://discord.gg/a', 'https://youtu.be/x']);
  });
});
