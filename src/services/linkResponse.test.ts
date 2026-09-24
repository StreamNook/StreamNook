import { describe, expect, it } from 'vitest';
import { classifyLinkResponse } from './linkResponse';

/**
 * Every one of these was a way for account linking to fail with no error
 * anywhere. The two "unavailable" cases are the ones that matter most: they are
 * what this site ACTUALLY answers for a route that is not deployed, measured
 * against production, and reading either as success would have a build that
 * shipped ahead of its server record nothing while believing it had.
 */
describe('classifyLinkResponse', () => {
  it('accepts the endpoint’s own success', () => {
    expect(classifyLinkResponse(200, true, '{"ok":true,"connected":true}')).toEqual({ kind: 'ok' });
  });

  it('reads an undeployed POST route (405) as unavailable, not failed', () => {
    // The live answer for POST /api/v1/accounts/link before it existed.
    expect(classifyLinkResponse(405, false, '')).toEqual({ kind: 'unavailable' });
  });

  it('reads the site’s HTML served with a 200 as unavailable, never as success', () => {
    // The SPA fallback serves index.html with a 200 for a path it does not know.
    const html = '<!doctype html><html lang="en"><head></head><body></body></html>';
    expect(classifyLinkResponse(200, true, html)).toEqual({ kind: 'unavailable' });
  });

  it('reads a 404 as unavailable', () => {
    expect(classifyLinkResponse(404, false, '{"error":"not found"}')).toEqual({ kind: 'unavailable' });
  });

  it('requires ok:true, not merely a 2xx with some JSON', () => {
    // A JSON body that is not the endpoint's success shape is not a success.
    expect(classifyLinkResponse(200, true, '{"something":"else"}')).toEqual({ kind: 'failed' });
    expect(classifyLinkResponse(200, true, '{"ok":false}')).toEqual({ kind: 'failed' });
  });

  it('turns a 409 into a conflict carrying the server’s retry hint', () => {
    expect(
      classifyLinkResponse(409, false, '{"error":"claim_too_recent","retry_after_minutes":7}'),
    ).toEqual({ kind: 'conflict', retryAfterMinutes: 7 });
  });

  it('keeps a 409 a conflict even without a usable retry hint', () => {
    expect(classifyLinkResponse(409, false, '{"error":"claimed_by_verified_owner"}')).toEqual({
      kind: 'conflict',
      retryAfterMinutes: null,
    });
    // A nonsense hint must not become "retry immediately, forever".
    expect(classifyLinkResponse(409, false, '{"retry_after_minutes":-3}')).toEqual({
      kind: 'conflict',
      retryAfterMinutes: null,
    });
  });

  it('reports a genuine server failure as failed', () => {
    expect(classifyLinkResponse(500, false, '{"error":"link_failed","detail":"x"}')).toEqual({ kind: 'failed' });
    expect(classifyLinkResponse(401, false, '{"error":"unauthorized"}')).toEqual({ kind: 'failed' });
  });

  it('does not mistake a JSON array for the endpoint’s object', () => {
    expect(classifyLinkResponse(200, true, '[1,2,3]')).toEqual({ kind: 'unavailable' });
  });
});
