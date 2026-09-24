import { describe, expect, it } from 'vitest';
import { computePaintStyleUncached, pickPaintLayerImage, type PaintV4 } from './paintStyle';

const BASE = 'https://cdn.7tv.app/paint/01K6ZYS5X6TF6EGK85Y9ZZR8YC/layer/01K6ZYS5X633HHK0K3JVDK4FT0';

function image(file: string, scale: number, frameCount: number) {
  return { url: `${BASE}/${file}`, mime: 'image/webp', size: 1, scale, width: 1, height: 1, frameCount };
}

// The order 7TV's paint catalog lists an animated layer in: every still
// `_static` variant first, then the animated files.
const animatedLayer = [
  image('1x_static.webp', 1, 1),
  image('2x_static.webp', 2, 1),
  image('1x_static.avif', 1, 1),
  image('1x.webp', 1, 24),
  image('2x.webp', 2, 24),
  image('1x.avif', 1, 24),
];

describe('pickPaintLayerImage', () => {
  it('picks the animated scale-1 file although 7TV lists the still first', () => {
    expect(pickPaintLayerImage(animatedLayer)?.url).toBe(`${BASE}/1x.webp`);
  });

  it('picks the first scale-1 file of a still layer', () => {
    const still = [image('2x.webp', 2, 1), image('1x.avif', 1, 1), image('1x.webp', 1, 1)];
    expect(pickPaintLayerImage(still)?.url).toBe(`${BASE}/1x.avif`);
  });

  it('picks nothing when the layer has no scale-1 file', () => {
    expect(pickPaintLayerImage([image('2x.webp', 2, 24)])).toBeUndefined();
  });
});

describe('computePaintStyleUncached', () => {
  it('draws the animated file of an animated image layer', () => {
    const paint: PaintV4 = {
      id: 'animated',
      name: 'animated',
      data: {
        layers: [{ id: 'layer', ty: { __typename: 'PaintLayerTypeImage', images: animatedLayer }, opacity: 1 }],
        shadows: [],
      },
    };
    expect(computePaintStyleUncached(paint).backgroundImage).toBe(`url("${BASE}/1x.webp")`);
  });
});
