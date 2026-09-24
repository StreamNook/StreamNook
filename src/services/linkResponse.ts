/**
 * What an answer from the account-link endpoint actually means.
 *
 * Kept apart from the service that makes the call so it can be tested on its
 * own, because every way this goes wrong is silent: a mis-read answer either
 * records nothing while reporting success, or reports a failure the member did
 * nothing to cause.
 */

/** What happened to a claim, as the caller needs to act on it. */
export type LinkWriteResult =
  | { kind: 'ok' }
  /** Someone else holds this account and their claim is too fresh to take. */
  | { kind: 'conflict'; retryAfterMinutes: number | null }
  /** The endpoint is not there (not deployed yet), or we could not reach it. */
  | { kind: 'unavailable' }
  | { kind: 'failed' };

/**
 * Classify one HTTP answer.
 *
 * Stricter than "status 2xx means it worked": a response only counts as the
 * endpoint's own if it is JSON carrying its `ok` field. That matters because of
 * what an undeployed route answers on this site — a POST gets 405, and an
 * unknown path can be served the website's own HTML with a 200 by the SPA
 * fallback. Neither is a claim being accepted, and reading the second one as
 * success would let a build that shipped ahead of its server record nothing
 * while believing it had.
 */
export function classifyLinkResponse(status: number, ok: boolean, body: string): LinkWriteResult {
  let json: Record<string, unknown> | null = null;
  try {
    const parsed: unknown = JSON.parse(body);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      json = parsed as Record<string, unknown>;
    }
  } catch {
    /* not JSON, handled below */
  }
  if (status === 404 || status === 405 || !json) return { kind: 'unavailable' };
  if (ok && json.ok === true) return { kind: 'ok' };
  if (status === 409) {
    const retry = json.retry_after_minutes;
    return {
      kind: 'conflict',
      retryAfterMinutes: typeof retry === 'number' && Number.isFinite(retry) && retry > 0 ? retry : null,
    };
  }
  return { kind: 'failed' };
}
