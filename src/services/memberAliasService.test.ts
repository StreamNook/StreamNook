import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const { normaliseResolution } = await import('./memberAliasService');

/**
 * "No claim" and "could not find out" must never collapse into one answer.
 * The first lets the chat stop asking about someone; the second has to be asked
 * again, or a single failed request leaves a member looking like a stranger for
 * the rest of the session.
 */
describe('normaliseResolution', () => {
  const keys = ['kick:1', 'youtube:UCabc'];

  it('passes the current shape through', () => {
    expect(
      normaliseResolution({ resolved: { 'kick:1': '249031143' }, unresolved: ['youtube:UCabc'] }, keys),
    ).toEqual({ resolved: { 'kick:1': '249031143' }, unresolved: ['youtube:UCabc'] });
  });

  it('keeps "no claim" distinct from "not answered"', () => {
    // Both keys asked, neither claimed, both answered: nothing to retry.
    expect(normaliseResolution({ resolved: {}, unresolved: [] }, keys)).toEqual({
      resolved: {},
      unresolved: [],
    });
  });

  it('reads an older backend’s bare map rather than throwing on it', () => {
    // A frontend hot-reloaded against an older Rust build gets this shape. It
    // used to be the whole contract, so it must still mean what it meant.
    expect(normaliseResolution({ 'kick:1': '249031143' }, keys)).toEqual({
      resolved: { 'kick:1': '249031143' },
      unresolved: [],
    });
  });

  it('treats anything unrecognisable as not answered, so it is asked again', () => {
    for (const bad of [null, undefined, 'oops', 42, [], [{ a: 1 }], { resolved: [] }, { 'kick:1': 7 }]) {
      expect(normaliseResolution(bad, keys)).toEqual({ resolved: {}, unresolved: keys });
    }
  });

  it('drops junk from the unresolved list instead of passing it along', () => {
    expect(
      normaliseResolution({ resolved: {}, unresolved: ['kick:1', 5, null, 'youtube:UCabc'] }, keys),
    ).toEqual({ resolved: {}, unresolved: ['kick:1', 'youtube:UCabc'] });
  });
});
