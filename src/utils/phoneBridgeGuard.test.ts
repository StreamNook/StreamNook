import { describe, expect, it } from 'vitest';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';

const ROOT = resolve(__dirname, '..', '..');
const GUARD = join(ROOT, 'scripts', 'phone-bridge-guard.sh');
const SHELL_FILE = join(ROOT, 'src', 'mobile', 'boot', 'useMobileBoot.ts');
const WIN = process.platform === 'win32';
const REFUSAL = 'phone link bridge is still in the tree';

// The directory of an rg on this machine. When there is none it throws, so a
// missing ripgrep fails the test instead of being read as a pass.
function rgDir(): string {
  const found = WIN
    ? spawnSync('where', ['rg'], { encoding: 'utf8' })
    : spawnSync('sh', ['-c', 'command -v rg'], { encoding: 'utf8' });
  const first =
    found.status === 0
      ? found.stdout.split(/\r?\n/).map((line) => line.trim()).find(Boolean)
      : undefined;
  if (!first) throw new Error('rg not found for the guard test');
  return dirname(first);
}

// Git for Windows' bash, never the bash.exe on PATH (that one can be WSL's).
function bashPath(): string {
  if (!WIN) return '/bin/bash';
  const roots = [
    process.env.ProgramFiles,
    process.env['ProgramFiles(x86)'],
    'C:\\Program Files',
    process.env.LOCALAPPDATA ? join(process.env.LOCALAPPDATA, 'Programs') : undefined,
  ];
  for (const root of roots) {
    if (!root) continue;
    const candidate = join(root, 'Git', 'bin', 'bash.exe');
    if (existsSync(candidate)) return candidate;
  }
  throw new Error('Git Bash not found for the guard test');
}

// Git Bash reads forward slashes in a Windows path; backslashes are escapes to it.
const forBash = (p: string) => (WIN ? p.replace(/\\/g, '/') : p);

function runGuard(target: string, withRg = true) {
  const env: NodeJS.ProcessEnv = { ...process.env };
  const key = Object.keys(env).find((k) => k.toLowerCase() === 'path') ?? 'PATH';
  env[key] = withRg ? `${rgDir()}${delimiter}${env[key] ?? ''}` : join(tmpdir(), 'no-such-bin-dir');
  const result = spawnSync(bashPath(), [forBash(GUARD), forBash(target)], {
    cwd: ROOT,
    env,
    encoding: 'utf8',
  });
  // 127 is "command not found". It is never a pass, so it is never accepted here.
  expect(result.status).not.toBe(127);
  return result;
}

function withTempFile(text: string, fn: (file: string) => void) {
  const dir = mkdtempSync(join(tmpdir(), 'bridge-guard-'));
  const file = join(dir, 'shell.ts');
  try {
    writeFileSync(file, text);
    fn(file);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const GUARD_BODY = [
  '#!/usr/bin/env bash',
  `rc=0; rg -q --fixed-strings 'PHONE_LINK_BRIDGE' "$1" || rc=$?; [ "$rc" -eq 1 ] || { echo "error: the phone link bridge is still in the tree; Android builds wait for the phone's own link doors" >&2; exit 1; }`,
].join('\n');

describe('the phone bridge guard', () => {
  // Fails if the guard passes a file that holds the marker.
  it('refuses a file that holds the marker', () => {
    withTempFile('export const PHONE_LINK_BRIDGE = true;\n', (file) => {
      const result = runGuard(file);
      expect(result.status).toBe(1);
      expect(result.stderr).toContain(REFUSAL);
    });
  }, 60_000);

  // Fails if the guard refuses every file (a broken script would also "refuse").
  it('passes a file without the marker', () => {
    withTempFile('export const OTHER = 1;\n', (file) => {
      const result = runGuard(file);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
    });
  }, 60_000);

  // Fails if only rg's "not found" exit counts as a refusal: rg exits 2 on an error.
  it('refuses when rg cannot read the file', () => {
    const result = runGuard(join(tmpdir(), 'bridge-guard-no-such-file.ts'));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(REFUSAL);
  }, 60_000);

  // Fails if a missing rg (127 inside the script) passes the build.
  it('refuses when rg is missing', () => {
    withTempFile('export const OTHER = 1;\n', (file) => {
      const result = runGuard(file, false);
      expect(result.status).toBe(1);
      expect(result.stderr).toContain(REFUSAL);
    });
  }, 60_000);

  it('refuses the real phone shell while it holds the marker', () => {
    expect(readFileSync(SHELL_FILE, 'utf8')).toContain('export const PHONE_LINK_BRIDGE = true;');
    const result = runGuard(SHELL_FILE);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(REFUSAL);
  }, 60_000);

  it('is exactly the checked-in script', () => {
    expect(readFileSync(GUARD, 'utf8').replace(/\r\n/g, '\n').trimEnd()).toBe(GUARD_BODY);
  });
});

describe('android-build.sh', () => {
  // Fails if the guard moves below the --debug branch or off the first line after the cd.
  it('runs the guard on the first line after its cd, ahead of the --debug branch', () => {
    const lines = readFileSync(join(ROOT, 'scripts', 'android-build.sh'), 'utf8').split(/\r?\n/);
    const cd = lines.findIndex((line) => line.trim() === 'cd "$(dirname "$0")/.."');
    expect(cd).toBeGreaterThan(-1);
    expect(lines[cd + 1]).toBe('bash scripts/phone-bridge-guard.sh src/mobile/boot/useMobileBoot.ts');
    const debug = lines.findIndex((line) => line.includes('"--debug"'));
    expect(debug).toBeGreaterThan(cd + 1);
  });
});
