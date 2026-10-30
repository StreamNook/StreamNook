import { describe, expect, it } from 'vitest';
import {
  AUTO_PUSH_PX,
  BRIGHTNESS_FLOOR,
  dialHundredth,
  dialTenth,
  layerBrightness,
  moveBrightnessDial,
  startBrightnessDial,
} from './brightnessDial';

// A travel of 300 px with a start of 0.5 puts the bottom of the dial 150 px
// below the finger, so Auto begins at 150 + 24 = 174 px of drag and ends again
// at 150 + 12 = 162 px.
const TRAVEL = 300;

describe('brightness dial', () => {
  it('steps on the tens digit of the hundredth on show', () => {
    expect(dialHundredth(0.504)).toBe(50);
    expect(dialTenth(0.296)).toBe(3);
    expect(dialTenth(0.294)).toBe(2);
  });

  it('turns the level up and down from where the finger landed', () => {
    const dial = startBrightnessDial(0.5, TRAVEL, false);
    expect(moveBrightnessDial(dial, -150, true).push).toBe(1);
    expect(moveBrightnessDial(dial, 30, true).push).toBeCloseTo(0.4, 10);
  });

  it('holds the floor, then reaches Auto past the bottom with the detent', () => {
    const dial = startBrightnessDial(0.5, TRAVEL, false);
    const floor = moveBrightnessDial(dial, 150, true);
    expect(floor.push).toBe(BRIGHTNESS_FLOOR);
    expect(floor.haptic).toBe('step');
    expect(floor.reading).toEqual({ axis: 'brightness', value: BRIGHTNESS_FLOOR });
    expect(moveBrightnessDial(dial, 150 + AUTO_PUSH_PX - 1, true).push).toBeNull();
    const auto = moveBrightnessDial(dial, 150 + AUTO_PUSH_PX, true);
    expect(auto.push).toBe(-1);
    expect(auto.haptic).toBe('detent');
    expect(auto.reading).toEqual({ axis: 'brightness', value: 0, auto: true });
  });

  it('never flickers under a finger resting at the detent', () => {
    const dial = startBrightnessDial(0.5, TRAVEL, false);
    expect(moveBrightnessDial(dial, 174, true).push).toBe(-1);
    expect(moveBrightnessDial(dial, 173, false).push).toBeNull();
    expect(moveBrightnessDial(dial, 174, false).push).toBeNull();
    const held = moveBrightnessDial(dial, 163, false);
    expect(held.push).toBeNull();
    expect(held.reading.auto).toBe(true);
    const back = moveBrightnessDial(dial, 162, false);
    expect(back.push).toBe(BRIGHTNESS_FLOOR);
    expect(back.haptic).toBe('step');
  });

  it('starts a swipe from Auto at the phone level without a tick', () => {
    const dial = startBrightnessDial(0.452, TRAVEL, true);
    const first = moveBrightnessDial(dial, 8, false);
    expect(first.push).toBeCloseTo(0.452 - 8 / TRAVEL, 10);
    expect(first.haptic).toBeNull();
    expect(moveBrightnessDial(dial, 30, true).haptic).toBe('step');
  });

  it('applies nothing when a swipe from Auto flicks straight into Auto', () => {
    const dial = startBrightnessDial(0.1, TRAVEL, true);
    const move = moveBrightnessDial(dial, 60, false);
    expect(move.push).toBeNull();
    expect(move.haptic).toBeNull();
    expect(move.reading).toEqual({ axis: 'brightness', value: 0, auto: true });
  });

  it('reads a start past full as full and a start under zero as zero', () => {
    const high = startBrightnessDial(8.03, TRAVEL, true);
    expect(moveBrightnessDial(high, 10, false).push).toBeCloseTo(1 - 10 / TRAVEL, 10);
    expect(moveBrightnessDial(high, 323, true).push).toBe(BRIGHTNESS_FLOOR);
    expect(moveBrightnessDial(high, 324, true).push).toBe(-1);
    const low = startBrightnessDial(-0.5, TRAVEL, false);
    expect(moveBrightnessDial(low, AUTO_PUSH_PX, true).push).toBe(-1);
  });

  it('ends the swipe when something else handed the screen back', () => {
    const dial = startBrightnessDial(0.5, TRAVEL, false);
    const first = moveBrightnessDial(dial, 30, true);
    expect(first.push).not.toBeNull();
    expect(first.stop).toBe(false);
    const gone = moveBrightnessDial(dial, 40, false);
    expect(gone.stop).toBe(true);
    expect(gone.push).toBeNull();
    expect(gone.haptic).toBeNull();
  });

  it('ends the swipe when the layer handed the screen back while the dial sat in Auto', () => {
    const dial = startBrightnessDial(0.5, TRAVEL, false, 0);
    expect(moveBrightnessDial(dial, 174, true, 0).push).toBe(-1);
    const gone = moveBrightnessDial(dial, 160, false, 1);
    expect(gone.stop).toBe(true);
    expect(gone.push).toBeNull();
  });
});

describe('layer brightness between swipes', () => {
  const level = () => 0.3;
  const watching = { watching: true, pip: false, visible: true };

  it('hands the screen back when playback ends or turns into an offline room', () => {
    expect(
      layerBrightness({ touched: true, saved: null }, { watching: false, pip: false, visible: true }, level),
    ).toEqual({ touched: false, saved: null, push: -1 });
  });

  it('sends nothing when no level was dialed', () => {
    const none = { touched: false, saved: null };
    expect(layerBrightness(none, { watching: false, pip: false, visible: true }, level).push).toBeNull();
    expect(layerBrightness(none, { watching: true, pip: true, visible: true }, level).push).toBeNull();
    expect(layerBrightness(none, watching, level).push).toBeNull();
  });

  it('sets a dialed level aside in PiP and brings it back after', () => {
    const pipped = layerBrightness({ touched: true, saved: null }, { watching: true, pip: true, visible: true }, level);
    expect(pipped).toEqual({ touched: false, saved: 0.3, push: -1 });
    expect(layerBrightness(pipped, watching, level)).toEqual({ touched: true, saved: null, push: 0.3 });
  });

  it('keeps the level set aside while the page is hidden', () => {
    const hidden = layerBrightness(
      { touched: false, saved: 0.3 },
      { watching: true, pip: false, visible: false },
      level,
    );
    expect(hidden).toEqual({ touched: false, saved: 0.3, push: null });
  });

  it('forgets the level set aside when playback ends in PiP', () => {
    const closed = layerBrightness({ touched: false, saved: 0.3 }, { watching: false, pip: true, visible: false }, level);
    expect(closed).toEqual({ touched: false, saved: null, push: null });
    expect(layerBrightness(closed, { watching: false, pip: false, visible: true }, level).push).toBeNull();
  });
});
