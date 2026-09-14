// Renders a MultiChat split tree (utils/splitTree.ts): nested flex rows and
// columns with draggable dividers, and a slim strip above every pane with the
// channel picker and split/close actions. Pure presenter: the tree is state
// owned by MultiChatWindow, every change goes back through `onChange`.

import { useCallback, useRef, type ReactNode } from 'react';
import { Columns2, Rows2, X } from 'lucide-react';
import { Dropdown, type DropdownOption } from '../ui/Dropdown';
import { Tooltip } from '../ui/Tooltip';
import {
  clampRatio,
  closeLeaf,
  setLeafKey,
  setRatio,
  splitLeaf,
  type SplitBranch,
  type SplitLeaf,
  type SplitNode,
} from '../../utils/splitTree';

export interface SplitLayoutProps {
  tree: SplitNode;
  onChange: (next: SplitNode | null) => void;
  /** Channels the picker can assign to a pane. */
  options: DropdownOption<string>[];
  /** Label for the "follow the active tab" choice. */
  activeLabel: string;
  focusedLeafId: string | null;
  onFocusLeaf: (id: string) => void;
  renderLeaf: (leaf: SplitLeaf) => ReactNode;
}

const FOLLOW = '__follow__';

export default function SplitLayout(props: SplitLayoutProps) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <Node node={props.tree} {...props} />
    </div>
  );
}

function Node({ node, ...ctx }: SplitLayoutProps & { node: SplitNode }) {
  if (node.kind === 'leaf') return <LeafSlot leaf={node} {...ctx} />;
  return <Branch branch={node} {...ctx} />;
}

function Branch({ branch, ...ctx }: SplitLayoutProps & { branch: SplitBranch }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const isRow = branch.dir === 'row';

  const onDividerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const el = containerRef.current;
      if (!el) return;
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const target = e.currentTarget;
      target.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => {
        const frac = isRow
          ? (ev.clientX - rect.left) / Math.max(1, rect.width)
          : (ev.clientY - rect.top) / Math.max(1, rect.height);
        ctx.onChange(setRatio(ctx.tree, branch.id, clampRatio(frac)));
      };
      const up = () => {
        target.removeEventListener('pointermove', move);
        target.removeEventListener('pointerup', up);
        target.removeEventListener('pointercancel', up);
      };
      target.addEventListener('pointermove', move);
      target.addEventListener('pointerup', up);
      target.addEventListener('pointercancel', up);
    },
    [branch.id, ctx, isRow],
  );

  return (
    <div ref={containerRef} className={`flex min-h-0 min-w-0 flex-1 ${isRow ? 'flex-row' : 'flex-col'}`}>
      <div className="flex min-h-0 min-w-0" style={{ flex: `${branch.ratio} 1 0%` }}>
        <Node node={branch.a} {...ctx} />
      </div>
      <div
        role="separator"
        aria-orientation={isRow ? 'vertical' : 'horizontal'}
        onPointerDown={onDividerDown}
        onDoubleClick={() => ctx.onChange(setRatio(ctx.tree, branch.id, 0.5))}
        className={`group relative z-10 flex flex-shrink-0 items-center justify-center ${
          isRow ? 'w-1 cursor-col-resize' : 'h-1 cursor-row-resize'
        }`}
        title="Drag to resize, double-click to even out"
      >
        <div
          className={`${isRow ? 'h-full w-px' : 'h-px w-full'} bg-borderSubtle transition-colors group-hover:bg-accent group-active:bg-accent`}
        />
      </div>
      <div className="flex min-h-0 min-w-0" style={{ flex: `${1 - branch.ratio} 1 0%` }}>
        <Node node={branch.b} {...ctx} />
      </div>
    </div>
  );
}

function LeafSlot({ leaf, ...ctx }: SplitLayoutProps & { leaf: SplitLeaf }) {
  const focused = ctx.focusedLeafId === leaf.id;
  const pickerOptions: DropdownOption<string>[] = [{ value: FOLLOW, label: ctx.activeLabel }, ...ctx.options];
  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      onMouseDownCapture={() => ctx.onFocusLeaf(leaf.id)}
    >
      <div
        className="sn-pane-strip flex h-[24px] flex-shrink-0 items-center justify-between gap-1 px-1"
        data-focused={focused ? 'true' : 'false'}
        data-tauri-drag-region="false"
      >
        <Dropdown
          value={leaf.key ?? FOLLOW}
          options={pickerOptions}
          onChange={(v) => ctx.onChange(setLeafKey(ctx.tree, leaf.id, v === FOLLOW ? null : String(v)))}
          className="h-[18px] max-w-[60%] px-1.5 text-[11px] text-textSecondary"
          align="left"
          ariaLabel="Channel for this pane"
        />
        <div className="flex items-center gap-0.5">
          <Tooltip content="Split right" side="bottom">
            <button
              type="button"
              onClick={() => ctx.onChange(splitLeaf(ctx.tree, leaf.id, 'row', null))}
              className="grid h-[18px] w-[18px] place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
              aria-label="Split right"
            >
              <Columns2 size={12} />
            </button>
          </Tooltip>
          <Tooltip content="Split down" side="bottom">
            <button
              type="button"
              onClick={() => ctx.onChange(splitLeaf(ctx.tree, leaf.id, 'col', null))}
              className="grid h-[18px] w-[18px] place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
              aria-label="Split down"
            >
              <Rows2 size={12} />
            </button>
          </Tooltip>
          <Tooltip content="Close pane" side="bottom">
            <button
              type="button"
              onClick={() => ctx.onChange(closeLeaf(ctx.tree, leaf.id))}
              className="grid h-[18px] w-[18px] place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-error"
              aria-label="Close pane"
            >
              <X size={12} />
            </button>
          </Tooltip>
        </div>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1">{ctx.renderLeaf(leaf)}</div>
    </div>
  );
}
