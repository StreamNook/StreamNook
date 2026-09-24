import { useState, type ReactNode } from 'react';
import {
  Bug,
  CircleArrowUp,
  Info,
  Palette,
  Puzzle,
  Sparkles,
  Trash2,
  Wrench,
  Zap,
  type LucideIcon,
} from 'lucide-react';
import { parseInlineMarkdown } from '../../services/markdownService';
import type {
  ChangelogItem as Item,
  ChangelogNode as Node,
  ChangelogSectionKind,
} from '../../types';

// Drawing only: Rust (`services::changelog`) fetched, parsed and grouped these
// nodes, and chose each section's kind.

type SectionNode = Extract<Node, { t: 'section' }>;

const SECTION_ICONS: Record<ChangelogSectionKind, LucideIcon> = {
  fixes: Bug,
  performance: Zap,
  maintenance: Wrench,
  removed: Trash2,
  plugins: Puzzle,
  interface: Palette,
  changes: CircleArrowUp,
  features: Sparkles,
  other: Info,
};

const sectionIcon = (kind: ChangelogSectionKind): ReactNode => {
  const Icon = SECTION_ICONS[kind] ?? Info;
  return <Icon size={14} className="text-accent shrink-0" />;
};

const SectionLabel = ({ label, kind }: { label: string; kind: ChangelogSectionKind }) => (
  <div className="flex items-center gap-2 px-1 mb-2.5 text-[13px] font-semibold text-textSecondary">
    {sectionIcon(kind)}
    <span>{parseInlineMarkdown(label)}</span>
  </div>
);

const Row = ({ item }: { item: Item }) => (
  <div className="px-4 py-3">
    {item.plain ? (
      <div className="text-[13.5px] leading-relaxed text-textSecondary">
        {parseInlineMarkdown(item.title)}
      </div>
    ) : (
      <>
        <div className="text-[14px] font-semibold text-textPrimary leading-snug">
          {parseInlineMarkdown(item.title)}
        </div>
        {item.desc && (
          <div className="mt-1 text-[13.5px] leading-relaxed text-textSecondary">
            {parseInlineMarkdown(item.desc)}
          </div>
        )}
      </>
    )}
  </div>
);

const Section = ({ node }: { node: SectionNode }) => (
  <section>
    {node.label && <SectionLabel label={node.label} kind={node.kind} />}
    {node.intro.map((p, i) => (
      <p key={i} className="px-1 mb-2.5 text-[13.5px] leading-relaxed text-textSecondary">
        {parseInlineMarkdown(p)}
      </p>
    ))}
    {node.items.length > 0 && (
      // Glass inside glass: the box rests IN the surface it sits on, so it
      // takes the glaze-inset lighting and no frost of its own. Rows sit
      // directly in it, split by hairlines, with no edge or fill of their own.
      <div className="glaze-inset hairline-y rounded-xl bg-white/[0.03]">
        {node.items.map((item, i) => (
          <Row key={i} item={item} />
        ))}
      </div>
    )}
  </section>
);

// A release image, often an animated WebP leading the notes. The picture runs
// edge to edge, which would bury a glaze rim drawn behind the content, so the
// rim is laid OVER the image instead: top light and a faint outline, with the
// contact shadow on the frame outside. A file that fails to load collapses
// rather than leaving a broken-image box at the top of the notes.
const ReleaseImage = ({ url, alt, eager }: { url: string; alt: string; eager: boolean }) => {
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  if (failed) return null;
  return (
    <div
      className="relative rounded-xl overflow-hidden bg-black/20"
      style={{
        boxShadow: 'var(--hairline-contour), 0 1px 0 0 rgba(0, 0, 0, 0.05), var(--elev-1)',
        // Release art is 16:9. Holding that slot until the file arrives keeps
        // the notes below from jumping down when a large WebP finishes loading.
        aspectRatio: loaded ? undefined : '16 / 9',
      }}
    >
      <img
        src={url}
        alt={alt}
        className="block w-full h-auto"
        // The lead image is the first thing on screen; lazy loading it only
        // delays the one picture everyone sees.
        loading={eager ? 'eager' : 'lazy'}
        decoding="async"
        onLoad={() => setLoaded(true)}
        onError={() => setFailed(true)}
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 rounded-[inherit]"
        style={{
          boxShadow:
            'inset 0 1px 0 0 rgba(255, 255, 255, 0.12), inset 0 0 0 1px rgba(255, 255, 255, 0.06)',
        }}
      />
    </div>
  );
};

const renderNode = (node: Node, key: number): ReactNode => {
  switch (node.t) {
    case 'hero':
      return (
        <div key={key} className="px-1">
          <div className="text-[19px] font-bold text-textPrimary leading-snug">
            {parseInlineMarkdown(node.title)}
          </div>
          {node.desc && (
            <p className="mt-1.5 text-[14px] leading-relaxed text-textSecondary">
              {parseInlineMarkdown(node.desc)}
            </p>
          )}
        </div>
      );
    case 'image':
      return <ReleaseImage key={key} url={node.url} alt={node.alt} eager={key === 0} />;
    case 'note':
      return (
        <p key={key} className="px-1 text-[13.5px] leading-relaxed text-textSecondary">
          {parseInlineMarkdown(node.desc)}
        </p>
      );
    case 'section':
      return <Section key={key} node={node} />;
  }
};

export const ReleaseNotes = ({ nodes }: { nodes: Node[] }) => {
  if (nodes.length === 0) {
    return <p className="px-1 text-[13px] italic text-textMuted">No release notes.</p>;
  }
  return <div className="flex flex-col gap-6 text-left">{nodes.map(renderNode)}</div>;
};
