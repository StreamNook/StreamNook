// Zoom to fill (SN-30): the fill scale, the pinch's preview, decision and tick,
// and the surface that is the zoom wrapper's one painter. The numbers come from
// the case table the iPhone's SNKit tests read too.
import { describe, expect, it } from 'vitest';
import cases from '../phoneGestureCases.json';
import {
  createZoomSurface,
  currentZoomSurface,
  fillScale,
  pinchDecision,
  pinchPreviewScale,
  pinchTick,
  registerZoomSurface,
} from './videoFill';

const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;
const target = () => ({ style: { transform: '', transition: '' } });

describe('fill scale', () => {
  it.each(cases.fill)('box $box with video $video scales $scale', ({ box, video, scale }) => {
    expect(near(fillScale(box[0], box[1], video[0], video[1]), scale)).toBe(true);
  });

  it('answers 1 for sizes that are not usable numbers', () => {
    expect(fillScale(NaN, 1080, 1920, 1080)).toBe(1);
    expect(fillScale(2400, 1080, Infinity, 1080)).toBe(1);
    expect(fillScale(2400, 1080, 1920, -1)).toBe(1);
  });
});

describe('pinch rules', () => {
  it.each(cases.preview)('preview from $startFill at $ratio is $scale', ({ startFill, fill, ratio, scale }) => {
    expect(near(pinchPreviewScale(startFill, fill, ratio), scale)).toBe(true);
  });

  it.each(cases.decision)('from $startFill at $ratio wants $want', ({ startFill, fill, ratio, want }) => {
    expect(pinchDecision(startFill, fill, ratio)).toBe(want);
  });

  it.each(cases.tick)('ticks once per crossing from $startFill', ({ startFill, fill, ratios, ticks }) => {
    let armed = true;
    const seen = ratios.map((r) => {
      const step = pinchTick(armed, startFill, fill, r);
      armed = step.armed;
      return step.tick;
    });
    expect(seen).toEqual(ticks);
  });
});

describe('zoom surface', () => {
  it('fills only while zoomable and wanted, and animates only a change of the choice', () => {
    const el = target();
    const s = createZoomSurface(el, () => undefined);
    s.setFill(1.25);
    expect(el.style.transform).toBe('');
    s.setInputs({ zoomable: true, want: false });
    expect(el.style.transform).toBe('');
    s.setInputs({ zoomable: true, want: true });
    expect(el.style.transform).toBe('scale(1.2500)');
    expect(el.style.transition).toContain('transform 220ms');
    s.setInputs({ zoomable: false, want: true });
    expect(el.style.transform).toBe('');
    expect(el.style.transition).toBe('none');
    s.setInputs({ zoomable: true, want: true });
    expect(el.style.transform).toBe('scale(1.2500)');
    expect(el.style.transition).toBe('none');
    expect(s.isOn()).toBe(true);
  });

  it('a preview leads until it ends, and a layout change drops it', () => {
    const el = target();
    const s = createZoomSurface(el, () => undefined);
    s.setFill(1.25);
    s.setInputs({ zoomable: true, want: false });
    s.preview(1.1);
    expect(el.style.transform).toBe('scale(1.1000)');
    expect(el.style.transition).toBe('none');
    s.endPreview();
    expect(el.style.transform).toBe('');
    expect(el.style.transition).toContain('transform 220ms');
    s.preview(1.3);
    s.setInputs({ zoomable: false, want: false });
    expect(el.style.transform).toBe('');
    el.style.transition = 'marker';
    s.endPreview();
    expect(el.style.transition).toBe('marker');
  });

  it('never animates under reduced or off motion', () => {
    for (const motion of ['reduced', 'off']) {
      const el = target();
      const s = createZoomSurface(el, () => motion);
      s.setFill(1.25);
      s.setInputs({ zoomable: true, want: false });
      s.setInputs({ zoomable: true, want: true });
      expect(el.style.transform).toBe('scale(1.2500)');
      expect(el.style.transition).toBe('none');
    }
  });

  it('a fill scale that is not a number paints Fit, and dispose clears the wrapper', () => {
    const el = target();
    const s = createZoomSurface(el, () => undefined);
    s.setInputs({ zoomable: true, want: true });
    s.setFill(NaN);
    expect(el.style.transform).toBe('');
    s.setFill(1.3);
    expect(el.style.transform).toBe('scale(1.3000)');
    s.dispose();
    expect(el.style.transform).toBe('');
    s.setFill(1.4);
    expect(el.style.transform).toBe('');
  });

  it('a late unregister never clears a newer surface', () => {
    const a = createZoomSurface(target(), () => undefined);
    const b = createZoomSurface(target(), () => undefined);
    const dropA = registerZoomSurface(a);
    const dropB = registerZoomSurface(b);
    dropA();
    expect(currentZoomSurface()).toBe(b);
    dropB();
    expect(currentZoomSurface()).toBeNull();
  });
});
