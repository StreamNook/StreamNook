import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// hls.js never gets promoted in-progress segments from the relay (only the
// iPhone's plain playlist asks for them), so the phone player must never take
// the promotion gap: on whole published segments it would sit about one
// segment from the end and stall.
const engine = readFileSync(fileURLToPath(new URL('./useMobileHlsEngine.ts', import.meta.url)), 'utf8');

describe('phone live path', () => {
  it('rides the parts or the plain gap, never promotion', () => {
    expect(engine).not.toContain('get_stream_prefetch_present');
    expect(engine).not.toContain("'promotion'");
  });

  it('learns its own cushion as the desktop player does', () => {
    expect(engine).toContain('learnedLLCushion');
    expect(engine).toContain('rememberLLCushion');
  });
});
