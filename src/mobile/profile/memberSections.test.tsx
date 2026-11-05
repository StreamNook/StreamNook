import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { MemberSections } from './MemberSections';
import { SAMPLE_MEMBER_PROFILE } from './profileSheetSample';

// React escapes the apostrophe in text; compare the plain words.
const draw = (accent?: string) =>
  renderToStaticMarkup(<MemberSections profile={SAMPLE_MEMBER_PROFILE} accent={accent} />).replace(/&#x27;/g, "'");

describe('a member sheet draws the desktop sections', () => {
  const out = draw('#8B5CF6');

  it('the hours hero: title, number, unit and roast', () => {
    expect(out).toContain('Hours watched');
    expect(out).toContain('>1,234<');
    expect(out).toContain('>hrs<');
    expect(out).toContain('That is 51 days of other people playing games.');
  });

  it('the hero heading is the label Rust sends, not a phone literal', () => {
    const relabeled = renderToStaticMarkup(
      <MemberSections
        profile={{ ...SAMPLE_MEMBER_PROFILE, hours: { ...SAMPLE_MEMBER_PROFILE.hours!, label: 'Time on stream' } }}
      />,
    );
    expect(relabeled).toContain('Time on stream');
    expect(relabeled).not.toContain('Hours watched');
  });

  it('both sections under Rust titles, tiles with captions', () => {
    expect(out).toContain("Tyler's Twitch");
    expect(out).toContain('Lifetime in StreamNook');
    expect(out).toContain('Cake day in 135 days');
    expect(out).toContain('into the void');
    expect(out).toContain('3 paints, 4 badges, 5 StreamNook');
  });

  it('the join date is not inline: Twitch age is the one tile that opens it in a panel', () => {
    expect(out).not.toContain('Joined March 20, 2014');
    expect(out.match(/aria-expanded="false"/g)?.length).toBe(1);
    expect(out).toContain('aria-label="Twitch age, 11 yrs, 7 mos"');
  });

  it('an odd last tile takes the whole row', () => {
    // Twitch has three tiles (Account type spans); Lifetime has four (none spans).
    expect(out.match(/col-span-2/g)?.length).toBe(1);
  });

  it('top emotes: counts, the totals line, and only the first tile lit', () => {
    expect(out).toContain('Top Emotes');
    expect(out).toContain('2 emotes · 1,350 used');
    expect(out).toContain('>900<');
    expect(out.match(/border-accent\/30/g)?.length).toBe(1);
  });

  it('the accolade wall: the count, earned medals only, the locked count on the expander', () => {
    expect(out).toContain('2 / 4 unlocked');
    expect(out).toContain('Chatty');
    expect(out).toContain('Night Owl');
    expect(out).not.toContain('Binge');
    expect(out).toContain('Show all 4 (2 locked)');
  });

  it('sections wear the accent border, and keep the theme border without one', () => {
    expect(out).toContain('border-color:rgba(139, 92, 246, 0.3)');
    expect(draw()).not.toContain('border-color');
  });

  it('draws no old Overview block', () => {
    expect(out).not.toContain('>Overview<');
  });
});
