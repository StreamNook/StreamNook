import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invokeSpy: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invokeSpy }));

import { categoryShareLinkFor, copyCategoryShareLink } from './shareLink';

beforeEach(() => {
  h.invokeSpy.mockReset();
});

describe('categoryShareLinkFor', () => {
  // Catches a pass-through of the whole category: a box art URL sent to Rust
  // would change the command's arguments.
  it('sends exactly the id and the name', async () => {
    h.invokeSpy.mockResolvedValue('https://streamnook.app/c/509658/rust');
    const category = { id: '509658', name: 'Rust', box_art_url: 'x' };
    const link = await categoryShareLinkFor(category);
    expect(link).toBe('https://streamnook.app/c/509658/rust');
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
    expect(h.invokeSpy).toHaveBeenCalledWith('category_share_link', { id: '509658', name: 'Rust' });
  });
});

describe('copyCategoryShareLink', () => {
  // Catches a copy that hands the writer something other than the built link,
  // or that reports success without writing.
  it('writes the built link and resolves true when both steps work', async () => {
    h.invokeSpy.mockResolvedValue('https://streamnook.app/c/1/a');
    const writeText = vi.fn().mockResolvedValue(undefined);
    await expect(copyCategoryShareLink({ id: '1', name: 'A' }, writeText)).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith('https://streamnook.app/c/1/a');
  });

  // Catches a refused id that rejects, or that still writes a clipboard value.
  it('resolves false, writes nothing and rejects nothing when the link is refused', async () => {
    h.invokeSpy.mockRejectedValue('not a Twitch category id');
    const writeText = vi.fn().mockResolvedValue(undefined);
    await expect(copyCategoryShareLink({ id: 'abc', name: 'A' }, writeText)).resolves.toBe(false);
    expect(writeText).not.toHaveBeenCalled();
  });

  // Catches a blocked clipboard that rejects instead of resolving false.
  it('resolves false when the clipboard write rejects', async () => {
    h.invokeSpy.mockResolvedValue('https://streamnook.app/c/1/a');
    const writeText = vi.fn().mockRejectedValue(new Error('denied'));
    await expect(copyCategoryShareLink({ id: '1', name: 'A' }, writeText)).resolves.toBe(false);
  });

  // Catches a writer that throws before returning a promise: the one try must
  // cover it too.
  it('resolves false when the writer throws synchronously', async () => {
    h.invokeSpy.mockResolvedValue('https://streamnook.app/c/1/a');
    const writeText = vi.fn(() => {
      throw new Error('illegal invocation');
    });
    await expect(copyCategoryShareLink({ id: '1', name: 'A' }, writeText)).resolves.toBe(false);
  });
});
