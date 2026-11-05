import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

describe('your wall comes from Rust', () => {
  it('the hook asks for it with the device offset and listens for changes', () => {
    const hook = read('src/hooks/useOwnAccolades.ts');
    expect(hook).toContain("'own_accolades', { tzOffsetMin: ownTzOffsetMin() }");
    // The saved wall is asked for first, then the fresh one.
    expect(hook.indexOf("'own_accolades_cached'")).toBeGreaterThan(-1);
    expect(hook.indexOf("'own_accolades_cached'")).toBeLessThan(hook.indexOf("'own_accolades', {"));
    expect(hook).toContain("'own-accolades-changed'");
  });

  it('the You tab draws it under the identity card for a signed-in account', () => {
    const you = read('src/mobile/screens/YouScreen.tsx');
    expect(you).toContain('{currentUser?.user_id && <OwnAccoladeSection userId={currentUser.user_id} />}');
    expect(you.indexOf('<OwnIdentityCard')).toBeLessThan(you.indexOf('<OwnAccoladeSection'));
  });

  it('the section draws the shared wall in a slot held at the shared layout height', () => {
    const section = read('src/mobile/profile/OwnAccoladeSection.tsx');
    expect(section).toContain('<AccoladeWall wall={wall} className="" />');
    expect(section).toContain('collapsedWallHeight()');
    expect(section).toContain('ownWallSlot(');
  });

  it('the wall takes its measurements from the layout, not from literals', () => {
    const wall = read('src/mobile/profile/AccoladeWall.tsx');
    for (const key of ['WALL.pad', 'WALL.header', 'WALL.rowGap', 'WALL.cell', 'WALL.sectionGap', 'WALL.toggle', 'WALL.progress']) {
      expect(wall, key).toContain(key);
    }
    expect(wall).toContain('accoladePanel(tapped)');
  });
});

describe('nothing in the webview saves an accolade itself', () => {
  // Written in pieces so this file does not name them.
  const gone = [
    ['grant', 'Accolade'],
    ['claim', 'LoginAccolades'],
    ['parse', 'Day'],
    ['isCake', 'DayOn'],
    ['seasonal', 'IdsOn'],
    ['seasonal', 'Accolades'],
  ].map((p) => p.join(''));

  it('no source names the old writer, the day claim or the dead seasonal helpers', () => {
    const sources = walk(resolve(root, 'src')).filter(
      (f) => !/[\\/]ownAccoladesWiring\.test\.ts$/.test(f) && !/[\\/]supabaseService\.profileChanged\.test\.ts$/.test(f),
    );
    for (const name of gone) {
      const offenders = sources.filter((f) => new RegExp(`\\b${name}\\b`).test(readFileSync(f, 'utf8')));
      expect(offenders, name).toEqual([]);
    }
  });

  it('the seasonal module and its test are deleted', () => {
    expect(existsSync(resolve(root, `src/utils/${gone[5]}.ts`))).toBe(false);
    expect(existsSync(resolve(root, `src/utils/${gone[5]}.test.ts`))).toBe(false);
  });

  it('the Android offset helper is the shared one', () => {
    const parts = read('src/mobile/profile/profileSheetParts.ts');
    expect(parts).toContain("export { ownTzOffsetMin as tzOffsetMin } from '../../hooks/useOwnAccolades';");
    expect(parts).not.toContain('getTimezoneOffset()');
  });

  it('Restless and the offline three go through the earner', () => {
    expect(read('src/utils/offlineAccolades.ts')).toContain('earnClientAccolade(id)');
    const notifications = read('src/components/settings/NotificationsSettings.tsx');
    expect(notifications).toContain('if (restlessReached(clicks)) void earnClientAccolade(RESTLESS_ACCOLADE_ID);');
    expect(notifications).not.toContain('=== NOTIF_CLICK_THRESHOLD');
  });

  it('the earner asks Rust for the signed-in account only: the id is the only argument', () => {
    expect(read('src/utils/earnAccolade.ts')).toContain("'grant_own_accolade', { id }");
  });
});
