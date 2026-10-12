// Run with: npm test
//
// Installed fonts on the overlay. CSS font-family matches an installed font by
// its family name only; the alias family's local() sources also find it by its
// face name or PostScript name, which is what font viewers and file names show.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { FONT_OPTIONS, localFontAlias, localFontFaceCss, overlayFontStack } from './overlayConfig';

test('a custom font gets the installed-font alias right after its name', () => {
  const alias = localFontAlias('Uni Sans Heavy Caps');
  assert.match(alias, /^sn-local-uni-sans-heavy-caps-[0-9a-z]+$/);
  assert.equal(
    overlayFontStack("'Uni Sans Heavy Caps', sans-serif"),
    `'Uni Sans Heavy Caps', '${alias}', sans-serif`,
  );
});

test('generic stacks are left alone', () => {
  assert.equal(overlayFontStack('system-ui, sans-serif'), 'system-ui, sans-serif');
  const system = FONT_OPTIONS.find((f) => f.label === 'System');
  assert.ok(system);
  assert.equal(overlayFontStack(system.value), system.value);
});

test('the alias looks the font up by face and PostScript names', () => {
  const css = localFontFaceCss('Montserrat Bold');
  assert.ok(css.startsWith(`@font-face{font-family:'${localFontAlias('Montserrat Bold')}';`));
  for (const name of ['Montserrat Bold', 'Montserrat Bold Regular', 'MontserratBold', 'MontserratBold-Regular']) {
    assert.ok(css.includes(`local('${name}')`), `missing local('${name}')`);
  }
});

test('names are escaped inside the CSS strings', () => {
  const css = localFontFaceCss("Bob's\\Font");
  assert.ok(css.includes("local('Bob\\'s\\\\Font')"), css);
});
