// Every bridge method the link doors call exists on both sides: in the page's
// SNBridge type and as a @JavascriptInterface method of MainActivity's
// InsetsBridge. A name on one side only fails silently on a phone: the page's
// optional call just does nothing.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (p: string) => readFileSync(resolve(__dirname, '..', '..', p), 'utf8').replace(/\r\n/g, '\n');

function between(text: string, from: string, to: string): string {
  const start = text.indexOf(from);
  const end = text.indexOf(to, start + from.length);
  return start >= 0 && end > start ? text.slice(start, end) : '';
}

const snBridge = between(read('src/mobile/nativeBridge.ts'), 'interface SNBridge {', '\n}\n');
const insetsBridge = between(
  read('src-tauri/gen/android/app/src/main/java/app/streamnook/MainActivity.kt'),
  'inner class InsetsBridge {',
  '\n  }\n',
);

const NAMES = [
  'consumePendingTap',
  'consumePendingChannelIf',
  'consumePendingMentionIf',
  'consumePendingWhisperIf',
  'bootNowMs',
  'isPipClosing',
];

describe('the link bridge names', () => {
  // A wrong path or a moved block would otherwise pass every name below.
  it('reads both sides', () => {
    expect(snBridge).toContain('  isInPip?(): boolean;');
    expect(insetsBridge).toContain('@JavascriptInterface\n    fun isInPip(): Boolean');
  });

  it.each(NAMES)('%s is in SNBridge and in InsetsBridge', (name) => {
    expect(snBridge).toContain(`  ${name}?(`);
    expect(insetsBridge).toContain(`@JavascriptInterface\n    fun ${name}(`);
  });
});
