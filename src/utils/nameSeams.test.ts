import { describe, expect, it } from 'vitest';
import { nameSeams } from './nameSeams';

describe('nameSeams', () => {
  it('splits a run-together name where its words meet', () => {
    expect(nameSeams('PrincessDonutBrown')).toEqual(['Princess', 'Donut', 'Brown']);
    expect(nameSeams('EnchantedBigBoiBoxers')).toEqual(['Enchanted', 'Big', 'Boi', 'Boxers']);
    expect(nameSeams('RoadToTwitchCon26')).toEqual(['Road', 'To', 'Twitch', 'Con', '26']);
  });

  it('keeps capital runs and a leading number whole', () => {
    expect(nameSeams('7TV Subscriber')).toEqual(['7TV Subscriber']);
    expect(nameSeams('Final Fantasy XIV')).toEqual(['Final Fantasy XIV']);
    expect(nameSeams('Gastly')).toEqual(['Gastly']);
  });

  it('joins back to the name unchanged', () => {
    for (const name of ['SuperUltraCombo 2023', 'BelieveInRichmond', '', 'a', "Belmont's Curse"]) {
      expect(nameSeams(name).join('')).toBe(name);
    }
  });
});
