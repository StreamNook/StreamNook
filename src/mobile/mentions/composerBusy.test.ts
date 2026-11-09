import { describe, expect, it } from 'vitest';
import { NO_COMPOSER_BUSY, withComposerBusy } from './composerBusy';

describe('withComposerBusy', () => {
  it('sets one chat and leaves the other', () => {
    expect(withComposerBusy(NO_COMPOSER_BUSY, 'chats', true)).toEqual({ watch: false, chats: true, home: false });
  });

  it('one chat clearing never clears the other', () => {
    expect(withComposerBusy({ watch: true, chats: true, home: false }, 'chats', false)).toEqual({ watch: true, chats: false, home: false });
  });

  it('no change gives null, so the store is not written', () => {
    expect(withComposerBusy(NO_COMPOSER_BUSY, 'watch', false)).toBeNull();
  });
});
