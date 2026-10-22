// Hover tooltips are the shared <Tooltip> (ui/Tooltip.tsx), drawn by the one
// TooltipManager main.tsx mounts for every window page. A native title= shows
// the operating system's tooltip instead: another look, its own delay, no
// edge flipping. It kept creeping back one attribute at a time, so this test
// reads the source and fails on any title= that reaches an element: on a DOM
// tag, or on a component whose props take the DOM attribute set and so pass
// it through (CardChip, ExternalAnchor and the like).
//
// `title` as a label prop (<SettingsRow title=...>) is not a tooltip and is
// not checked. Phones have no hover, so src/mobile is skipped. A deliberate
// native title carries `tooltip-exempt: <reason>` in a comment within the
// four lines above it.
//
// Also held: TooltipManager is mounted once, in main.tsx. A second mount
// draws every tooltip twice; a window page with none draws nothing.
import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';
import ts from 'typescript';

const src = resolve(__dirname, '..', '..');
const SKIP_DIRS = new Set(['mobile']);
// Props types that take the DOM attribute set.
const DOM_PROPS = /\b(?:[A-Za-z]*HTMLAttributes|ComponentProps(?:WithoutRef|WithRef)?<\s*['"][a-z]+['"]|SVGProps)\b/;

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (!SKIP_DIRS.has(name)) walk(p, out);
    } else if (p.endsWith('.tsx') && !p.endsWith('.test.tsx')) {
      out.push(p);
    }
  }
  return out;
}
const rel = (p: string) => relative(src, p).split(sep).join('/');
const parsed = walk(src).map((file) => {
  const text = readFileSync(file, 'utf8');
  return { file, text, sf: ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX) };
});

/** Components whose props take DOM attributes without leaving out `title`. */
function domForwarders(): Set<string> {
  const names = new Set<string>();
  for (const { sf } of parsed) {
    const consider = (name: string, fn: ts.SignatureDeclaration, extra: string) => {
      const first = fn.parameters[0];
      if (!/^[A-Z]/.test(name) || !first) return;
      let typeText = `${first.type?.getText(sf) ?? ''} ${extra}`;
      // Props types declared in the same file, one level deep.
      for (const id of new Set(typeText.match(/\b[A-Z]\w*/g) ?? [])) {
        for (const s of sf.statements) {
          if ((ts.isInterfaceDeclaration(s) || ts.isTypeAliasDeclaration(s)) && s.name.text === id) {
            typeText += ` ${s.getText(sf)}`;
          }
        }
      }
      if (DOM_PROPS.test(typeText) && !/Omit<[^>]*['"]title['"]/.test(typeText)) names.add(name);
    };
    const visit = (node: ts.Node) => {
      if (ts.isFunctionDeclaration(node) && node.name) consider(node.name.text, node, '');
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
        // React.FC<Props> on the variable; forwardRef<El, Props>(...) or memo(...) around the function.
        let init: ts.Expression = node.initializer;
        let extra = node.type?.getText(sf) ?? '';
        while (ts.isCallExpression(init) && init.arguments[0]) {
          extra += ` ${init.typeArguments?.map((t) => t.getText(sf)).join(' ') ?? ''}`;
          init = init.arguments[0];
        }
        if (ts.isArrowFunction(init) || ts.isFunctionExpression(init)) consider(node.name.text, init, extra);
      }
      ts.forEachChild(node, visit);
    };
    visit(sf);
  }
  return names;
}
const forwarders = domForwarders();

function nativeTitles({ file, text, sf }: (typeof parsed)[number]): string[] {
  const lines = text.split('\n');
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isJsxAttribute(node) && node.name.getText(sf) === 'title') {
      const tag = (node.parent.parent as ts.JsxOpeningLikeElement).tagName.getText(sf);
      const line = sf.getLineAndCharacterOfPosition(node.getStart(sf)).line;
      const exempt = lines.slice(Math.max(0, line - 4), line + 1).some((l) => l.includes('tooltip-exempt:'));
      if ((/^[a-z]/.test(tag) || forwarders.has(tag)) && !exempt) found.push(`${rel(file)}:${line + 1} <${tag} title>`);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return found;
}

describe('tooltips', () => {
  it('no element gets a native title= tooltip', () => {
    expect(parsed.flatMap(nativeTitles)).toEqual([]);
  });

  it('still recognises the components that pass DOM attributes through', () => {
    expect([...forwarders]).toEqual(expect.arrayContaining(['CardChip', 'ExternalAnchor', 'FallbackImage', 'SevenTvAvatar']));
  });

  it('TooltipManager is mounted once, in main.tsx', () => {
    const mounts = parsed.filter(({ text }) => /<TooltipManager\s*\/>/.test(text)).map(({ file }) => rel(file));
    expect(mounts).toEqual(['main.tsx']);
  });
});
