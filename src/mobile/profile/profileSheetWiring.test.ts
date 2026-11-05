// The Android sheet's wiring, pinned in source: what it sends Rust, when it
// reads, and which pieces it draws. (It reads other files, never its own.)
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');
const sheet = read('src/mobile/profile/UserProfileSheet.tsx');

describe('the sheet asks Rust with a per-presentation id and the device offset', () => {
  it('sends openId on the cached read', () => {
    expect(sheet).toMatch(/'user_profile_sheet_cached',\s*\{[^}]*\bopenId\b[^}]*\}/);
  });

  it('sends openId and the offset on the fresh read', () => {
    expect(sheet).toMatch(/'user_profile_sheet',\s*\{[^}]*tzOffsetMin: tzOffsetMin\(\)[^}]*\bopenId\b[^}]*\}/);
  });

  it('mints the id when the sheet opens and reads nothing while closed', () => {
    expect(sheet).toContain('setOpenId(newOpenId())');
    expect(sheet).toContain('if (!open) return;');
    expect(sheet).toContain('[open, openId, userId');
  });

  it('lets go of a held plate slot even when the fresh read fails', () => {
    expect(sheet).toMatch(/catch \(err\) \{[\s\S]*?setFreshAnswered\(true\)/);
  });
});

describe('the sheet draws the new pieces', () => {
  it('the plate, the caption, the relic showcase and the whole-sheet backdrop', () => {
    expect(sheet).toContain('<ProfileNameplate slot={slot} link={pageLink} size="phone" />');
    expect(sheet).toContain('<MemberRankMark rank={sheet.member} caption={sheet.caption} />');
    expect(sheet).toContain('<RelicShowcase');
    expect(sheet).toContain('backdrop={backdrop}');
    expect(sheet).not.toContain('AtmosphereGlow');
  });

  it('puts a tap panel away with the shared outside-press hook', () => {
    expect(read('src/mobile/profile/IdentityParts.tsx')).toMatch(/useOutsidePress\(open, \[host\], onClose\)/);
    expect(read('src/mobile/profile/ProfileHeroRows.tsx')).toContain('<TapPanelHost');
    expect(read('src/mobile/profile/AccoladeWall.tsx')).toContain('<TapPanelHost');
    expect(read('src/mobile/profile/MemberSections.tsx')).toContain('<TapPanelHost');
  });

  it('the group badge panel closes on an outside tap too', () => {
    expect(sheet).toContain('<TapPanelHost open={panelOpen} onClose={onPanelClose}>');
    expect(sheet).toContain('panelOpen={picked?.group === title}');
    expect(sheet).toContain('onPanelClose={() => setPicked(null)}');
  });

  it('reads no catalog JSON on the phone', () => {
    for (const f of ['UserProfileSheet', 'MemberSections', 'AccoladeWall', 'RelicShowcase', 'profileSheetParts', 'SheetBackdrop']) {
      expect(read(`src/mobile/profile/${f}.ts${f === 'profileSheetParts' ? '' : 'x'}`)).not.toMatch(/from '[^']*accolades\.catalog/);
    }
  });

  it('keeps the wall from anchoring the scroll while it grows', () => {
    expect(read('src/mobile/profile/AccoladeWall.tsx')).toContain('[overflow-anchor:none]');
  });

  it('your own card draws the plate from the saved or fresh identity', () => {
    const own = read('src/mobile/profile/OwnIdentityHeader.tsx');
    expect(own).toContain('identity?.page_link');
    expect(own).toContain('<ProfileNameplate slot={slot} link={pageLink} size="phone" />');
  });
});

describe('the sheet motion', () => {
  it('adds no loop of its own: the backdrop reuses the classes that rest under low power and Motion Off', () => {
    const backdrop = read('src/mobile/profile/SheetBackdrop.tsx');
    expect(backdrop).not.toMatch(/animate-|@keyframes|infinite/);
    expect(backdrop).toContain('sn-atm-breathe-1');
  });

  it('the Atmosphere sits under the 0.58 scrim with no 44 px blur, and the paint wash is held still', () => {
    const backdrop = read('src/mobile/profile/SheetBackdrop.tsx');
    expect(backdrop).toContain("'rgba(10,10,14,0.58)'");
    expect(backdrop).not.toContain('44px');
    expect(backdrop).not.toContain('backdrop-filter');
    expect(backdrop).toContain('opacity: 0.2');
    // The rank aura is the only layer that breathes.
    expect(backdrop.match(/sn-atm-breathe-1/g)?.length).toBe(2); // the header comment and the aura layer
  });
});
