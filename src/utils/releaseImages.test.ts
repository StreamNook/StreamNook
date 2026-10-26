import { describe, expect, it } from 'vitest';
import { releaseImage, releaseImagesIn, releaseRowImagesOnUnmount } from './releaseImages';

/** An <img> stand-in that keeps its attributes in a map. */
function fakeImg(attrs: Record<string, string>) {
  const map = new Map(Object.entries(attrs));
  return {
    map,
    removeAttribute: (name: string) => {
      map.delete(name);
    },
  };
}

/** A row stand-in: connected or not, holding the images below it. */
function fakeRow(imgs: ReturnType<typeof fakeImg>[], connected: boolean) {
  const row = {
    isConnected: connected,
    querySelectorAll: (selector: string) => (selector === 'img' ? imgs : []),
  };
  return row;
}

const asImg = (i: ReturnType<typeof fakeImg>) => i as unknown as HTMLImageElement;
const asRow = (r: ReturnType<typeof fakeRow>) => r as unknown as HTMLElement;

describe('releaseImage', () => {
  it('removes srcset and src', () => {
    const img = fakeImg({ src: 'a.webp', srcset: 'a.webp 1x, b.webp 2x', alt: 'x' });
    releaseImage(asImg(img));
    expect(img.map.has('src')).toBe(false);
    expect(img.map.has('srcset')).toBe(false);
    expect(img.map.get('alt')).toBe('x');
  });

  it('is fine on an image with neither', () => {
    const img = fakeImg({});
    expect(() => releaseImage(asImg(img))).not.toThrow();
  });
});

describe('releaseImagesIn', () => {
  it('releases every image under the root', () => {
    const a = fakeImg({ src: 'a' });
    const b = fakeImg({ src: 'b', srcset: 'b 2x' });
    releaseImagesIn(asRow(fakeRow([a, b], false)) as unknown as ParentNode);
    expect(a.map.size).toBe(0);
    expect(b.map.size).toBe(0);
  });
});

describe('releaseRowImagesOnUnmount', () => {
  it('releases after a microtask when the row has left the document', async () => {
    const img = fakeImg({ src: 'a' });
    const cleanup = releaseRowImagesOnUnmount(asRow(fakeRow([img], false)));
    expect(cleanup).toBeTypeOf('function');
    cleanup?.();
    expect(img.map.has('src')).toBe(true);
    await Promise.resolve();
    expect(img.map.has('src')).toBe(false);
  });

  it('keeps the images while the row is still connected', async () => {
    const img = fakeImg({ src: 'a' });
    releaseRowImagesOnUnmount(asRow(fakeRow([img], true)))?.();
    await Promise.resolve();
    expect(img.map.has('src')).toBe(true);
  });

  it('gives no cleanup for null', () => {
    expect(releaseRowImagesOnUnmount(null)).toBeUndefined();
  });
});
