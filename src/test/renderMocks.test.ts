// Run with: npm test
//
// Every render test mocks the platform module by its path relative to the test file ('../utils/platform'
// from src/components/, '../../utils/platform' from src/mobile/screens/). A path that resolves anywhere
// else mocks nothing, the real module reads the host's user agent, and the test passes on one machine
// and fails on another. This resolves every such mock path to src/utils/platform.ts.
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const ROOT = resolve(__dirname, '..', '..');
const SRC = join(ROOT, 'src');
const PLATFORM = join(SRC, 'utils', 'platform');
const RENDER_HEADER = '// @vitest-environment happy-dom';

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(full));
    else if (/\.test\.tsx?$/.test(entry.name)) out.push(full);
  }
  return out;
}

/** Whether a test file declares the happy-dom environment on its first line. */
function isRenderTest(text: string): boolean {
  return text.split(/\r?\n/, 1)[0].trim() === RENDER_HEADER;
}

/** The path of every vi.mock call that names a module called "platform". */
function platformMockPaths(text: string): string[] {
  return [...text.matchAll(/vi\.mock\(\s*(['"])([^'"]*\/platform(?:\.ts)?)\1/g)].map((m) => m[2]);
}

/** True when `mockPath`, written in `file`, names src/utils/platform. */
function resolvesToPlatform(file: string, mockPath: string): boolean {
  return resolve(dirname(file), mockPath).replace(/\.ts$/, '') === PLATFORM;
}

const renderTests = walk(SRC).filter((file) => isRenderTest(readFileSync(file, 'utf8')));

describe('render test platform mocks', () => {
  it('scans at least one render test, so a wrong path or an empty walk fails', () => {
    expect(renderTests.length).toBeGreaterThan(0);
    expect(renderTests.some((file) => file.endsWith(join('src', 'test', 'renderEnv.test.tsx')))).toBe(true);
  });

  it('every platform mock in a render test resolves to src/utils/platform.ts', () => {
    const wrong: string[] = [];
    let mocks = 0;
    for (const file of renderTests) {
      for (const mockPath of platformMockPaths(readFileSync(file, 'utf8'))) {
        mocks += 1;
        if (!resolvesToPlatform(file, mockPath)) wrong.push(`${file}: ${mockPath}`);
      }
    }
    expect(mocks).toBeGreaterThan(0);
    expect(wrong).toEqual([]);
  });

  // Positive control: the check can fail. One folder short resolves to src/platform, not src/utils/platform.
  it('flags a mock path that resolves elsewhere', () => {
    const component = join(SRC, 'components', 'Probe.test.tsx');
    const screen = join(SRC, 'mobile', 'screens', 'Probe.test.tsx');
    expect(resolvesToPlatform(component, '../utils/platform')).toBe(true);
    expect(resolvesToPlatform(component, '../platform')).toBe(false);
    expect(resolvesToPlatform(screen, '../../utils/platform')).toBe(true);
    expect(resolvesToPlatform(screen, '../utils/platform')).toBe(false);
  });

  it('reads the mock call from source text, in either quote style', () => {
    const text = [
      "vi.mock('../utils/platform', () => ({}));",
      'vi.mock("../../utils/platform.ts", () => ({}));',
      "vi.mock('../other', () => ({}));",
    ].join('\n');
    expect(platformMockPaths(text)).toEqual(['../utils/platform', '../../utils/platform.ts']);
  });

  it('knows a render test by its first line only', () => {
    expect(isRenderTest('// @vitest-environment happy-dom\nimport x')).toBe(true);
    expect(isRenderTest('// @vitest-environment happy-dom\r\nimport x')).toBe(true);
    expect(isRenderTest('import x\n// @vitest-environment happy-dom')).toBe(false);
  });
});
