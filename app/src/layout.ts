export type TabRef = { kind: 'session'; deck_id: string } | { kind: 'changes'; project_id: string };
export interface Pane {
  type: 'pane';
  id: string;
  tabs: TabRef[];
  active: number;
}
export interface Split {
  type: 'split';
  dir: 'row' | 'column';
  ratio: number;
  a: LayoutNode;
  b: LayoutNode;
}
export type LayoutNode = Pane | Split;
export type Edge = 'left' | 'right' | 'top' | 'bottom';
export type DropZone = Edge | 'center';
export type Path = ('a' | 'b')[];

export const MAX_PANES = 4;
export const MIN_RATIO = 0.15;
export const MAX_RATIO = 0.85;

const EPS = 1e-6;

export function tabKey(t: TabRef): string {
  return t.kind === 'session' ? `session:${t.deck_id}` : `changes:${t.project_id}`;
}

export function emptyLayout(newId: () => string): Pane {
  return { type: 'pane', id: newId(), tabs: [], active: 0 };
}

export function panes(n: LayoutNode): Pane[] {
  return n.type === 'pane' ? [n] : [...panes(n.a), ...panes(n.b)];
}

export function findPane(n: LayoutNode, paneId: string): Pane | undefined {
  return panes(n).find((p) => p.id === paneId);
}

export function findTab(n: LayoutNode, key: string): { pane: Pane; index: number } | undefined {
  for (const pane of panes(n)) {
    const index = pane.tabs.findIndex((t) => tabKey(t) === key);
    if (index >= 0) return { pane, index };
  }
  return undefined;
}

export function allTabs(n: LayoutNode): TabRef[] {
  return panes(n).flatMap((p) => p.tabs);
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function mapPanes(n: LayoutNode, fn: (p: Pane) => Pane): LayoutNode {
  if (n.type === 'pane') return fn(n);
  return { ...n, a: mapPanes(n.a, fn), b: mapPanes(n.b, fn) };
}

/** Removes tabs matching `drop` from a pane, keeping the active tab when it survives. */
function withoutTabs(p: Pane, drop: (t: TabRef) => boolean): Pane {
  if (!p.tabs.some(drop)) return p;
  const activeTab = p.tabs[p.active];
  const tabs = p.tabs.filter((t) => !drop(t));
  let active = activeTab && !drop(activeTab) ? tabs.indexOf(activeTab) : -1;
  if (active < 0) {
    const removedBefore = p.tabs.slice(0, p.active).filter(drop).length;
    active = clamp(p.active - removedBefore, 0, Math.max(0, tabs.length - 1));
  }
  return { ...p, tabs, active };
}

/** Removes empty panes; a split with a vanished child is replaced by the other child. */
function collapse(n: LayoutNode): LayoutNode | undefined {
  if (n.type === 'pane') return n.tabs.length === 0 ? undefined : n;
  const a = collapse(n.a);
  const b = collapse(n.b);
  if (a && b) return a === n.a && b === n.b ? n : { ...n, a, b };
  return a ?? b;
}

/** Like collapse, but never leaves the root without a pane. */
function collapseKeepRoot(n: LayoutNode): LayoutNode {
  const r = collapse(n);
  if (r) return r;
  const first = panes(n)[0];
  return first ? { ...first, tabs: [], active: 0 } : n;
}

function removeTabEverywhere(n: LayoutNode, key: string): LayoutNode {
  return mapPanes(n, (p) => withoutTabs(p, (t) => tabKey(t) === key));
}

function performDrop(
  n: LayoutNode,
  targetPaneId: string,
  zone: DropZone,
  tab: TabRef,
  newId: () => string,
): LayoutNode | undefined {
  if (!findPane(n, targetPaneId)) return undefined;
  const key = tabKey(tab);
  if (zone === 'center' && findTab(n, key)?.pane.id === targetPaneId) return undefined;
  const removed = removeTabEverywhere(n, key);
  let next: LayoutNode;
  if (zone === 'center') {
    next = mapPanes(removed, (p) =>
      p.id === targetPaneId ? { ...p, tabs: [...p.tabs, tab], active: p.tabs.length } : p,
    );
  } else {
    const fresh: Pane = { type: 'pane', id: newId(), tabs: [tab], active: 0 };
    const dir = zone === 'left' || zone === 'right' ? 'row' : 'column';
    const first = zone === 'left' || zone === 'top';
    const replace = (node: LayoutNode): LayoutNode => {
      if (node.type === 'pane') {
        if (node.id !== targetPaneId) return node;
        return { type: 'split', dir, ratio: 0.5, a: first ? fresh : node, b: first ? node : fresh };
      }
      return { ...node, a: replace(node.a), b: replace(node.b) };
    };
    next = replace(removed);
  }
  return collapseKeepRoot(next);
}

export function canDrop(n: LayoutNode, targetPaneId: string, zone: DropZone, tab: TabRef): boolean {
  const r = performDrop(n, targetPaneId, zone, tab, () => '__probe__');
  return r !== undefined && panes(r).length <= MAX_PANES;
}

export function dropTab(
  n: LayoutNode,
  targetPaneId: string,
  zone: DropZone,
  tab: TabRef,
  newId: () => string,
): LayoutNode {
  const r = performDrop(n, targetPaneId, zone, tab, newId);
  return r !== undefined && panes(r).length <= MAX_PANES ? r : n;
}

export function addTab(n: LayoutNode, paneId: string, tab: TabRef): LayoutNode {
  const target = findPane(n, paneId);
  if (!target) return n;
  const key = tabKey(tab);
  const existing = target.tabs.findIndex((t) => tabKey(t) === key);
  if (existing >= 0) return setActive(n, paneId, existing);
  const removed = removeTabEverywhere(n, key);
  const added = mapPanes(removed, (p) =>
    p.id === paneId ? { ...p, tabs: [...p.tabs, tab], active: p.tabs.length } : p,
  );
  return collapseKeepRoot(added);
}

export function closeTab(n: LayoutNode, key: string): LayoutNode {
  if (!findTab(n, key)) return n;
  return collapseKeepRoot(removeTabEverywhere(n, key));
}

export function setActive(n: LayoutNode, paneId: string, index: number): LayoutNode {
  return mapPanes(n, (p) =>
    p.id === paneId ? { ...p, active: clamp(index, 0, Math.max(0, p.tabs.length - 1)) } : p,
  );
}

export function setRatio(n: LayoutNode, path: Path, ratio: number): LayoutNode {
  if (n.type !== 'split') return n;
  const [head, ...rest] = path;
  if (head === undefined) return { ...n, ratio: clamp(ratio, MIN_RATIO, MAX_RATIO) };
  const child = setRatio(n[head], rest, ratio);
  if (child === n[head]) return n;
  return { ...n, [head]: child };
}

export function prune(n: LayoutNode, keep: (t: TabRef) => boolean): LayoutNode {
  return collapseKeepRoot(mapPanes(n, (p) => withoutTabs(p, (t) => !keep(t))));
}

export function splitPane(n: LayoutNode, paneId: string, dir: 'row' | 'column', newId: () => string): LayoutNode {
  const p = findPane(n, paneId);
  if (!p || p.tabs.length < 2) return n;
  const tab = p.tabs[p.active];
  if (!tab) return n;
  return dropTab(n, paneId, dir === 'row' ? 'right' : 'bottom', tab, newId);
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function layoutRects(n: LayoutNode, r: Rect, out: Map<string, Rect>): void {
  if (n.type === 'pane') {
    out.set(n.id, r);
    return;
  }
  if (n.dir === 'row') {
    const wa = r.w * n.ratio;
    layoutRects(n.a, { ...r, w: wa }, out);
    layoutRects(n.b, { ...r, x: r.x + wa, w: r.w - wa }, out);
  } else {
    const ha = r.h * n.ratio;
    layoutRects(n.a, { ...r, h: ha }, out);
    layoutRects(n.b, { ...r, y: r.y + ha, h: r.h - ha }, out);
  }
}

function overlap(a0: number, a1: number, b0: number, b1: number): number {
  return Math.min(a1, b1) - Math.max(a0, b0);
}

export function neighbour(
  n: LayoutNode,
  paneId: string,
  dir: 'left' | 'right' | 'up' | 'down',
): string | undefined {
  const rects = new Map<string, Rect>();
  layoutRects(n, { x: 0, y: 0, w: 1, h: 1 }, rects);
  const cur = rects.get(paneId);
  if (!cur) return undefined;
  let best: string | undefined;
  let bestOverlap = EPS;
  for (const p of panes(n)) {
    const r = rects.get(p.id);
    if (p.id === paneId || !r) continue;
    let touches: boolean;
    let ov: number;
    if (dir === 'left') touches = Math.abs(r.x + r.w - cur.x) < EPS;
    else if (dir === 'right') touches = Math.abs(r.x - (cur.x + cur.w)) < EPS;
    else if (dir === 'up') touches = Math.abs(r.y + r.h - cur.y) < EPS;
    else touches = Math.abs(r.y - (cur.y + cur.h)) < EPS;
    if (!touches) continue;
    if (dir === 'left' || dir === 'right') ov = overlap(cur.y, cur.y + cur.h, r.y, r.y + r.h);
    else ov = overlap(cur.x, cur.x + cur.w, r.x, r.x + r.w);
    if (ov > bestOverlap) {
      bestOverlap = ov;
      best = p.id;
    }
  }
  return best;
}

const CANVAS_W = 1.6;

/** Largest pane on a wide canvas; ties go to the later pane in panes() order. */
function largestPane(n: LayoutNode): { pane: Pane; rect: Rect } {
  const rects = new Map<string, Rect>();
  layoutRects(n, { x: 0, y: 0, w: CANVAS_W, h: 1 }, rects);
  let best: { pane: Pane; rect: Rect } | undefined;
  for (const p of panes(n)) {
    const rect = rects.get(p.id);
    if (!rect) continue;
    if (!best || rect.w * rect.h >= best.rect.w * best.rect.h - EPS) best = { pane: p, rect };
  }
  return best as { pane: Pane; rect: Rect };
}

/** Adds `tab` in its own new pane (auto-tiling) when the tree has fewer than MAX_PANES panes;
 *  otherwise adds it as a tab to the largest pane. If the tab is already in the tree, it is moved (same rules).
 *  An empty root pane (no tabs) is filled instead of split.
 *  Tiling rule: pick the pane with the largest area (ties -> the later pane in panes() order), computed on a
 *  1.6 : 1 (wide) canvas; split it 'row' if its width >= its height, else 'column'; the new pane goes second.
 *  Sequence from one pane: 2 -> side by side; 3 -> left full + right stacked; 4 -> 2x2. */
export function addTabTiled(n: LayoutNode, tab: TabRef, newId: () => string): LayoutNode {
  const base = findTab(n, tabKey(tab)) ? collapseKeepRoot(removeTabEverywhere(n, tabKey(tab))) : n;
  if (base.type === 'pane' && base.tabs.length === 0) return { ...base, tabs: [tab], active: 0 };
  const { pane: target, rect } = largestPane(base);
  if (panes(base).length >= MAX_PANES) {
    return mapPanes(base, (p) => (p.id === target.id ? { ...p, tabs: [...p.tabs, tab], active: p.tabs.length } : p));
  }
  const fresh: Pane = { type: 'pane', id: newId(), tabs: [tab], active: 0 };
  const dir = rect.w >= rect.h - EPS ? 'row' : 'column';
  const replace = (node: LayoutNode): LayoutNode => {
    if (node.type === 'pane') {
      return node.id === target.id ? { type: 'split', dir, ratio: 0.5, a: node, b: fresh } : node;
    }
    return { ...node, a: replace(node.a), b: replace(node.b) };
  };
  return replace(base);
}

/** Re-arranges every tab of the tree with addTabTiled, starting from an empty pane, keeping tab order:
 *  up to 4 tabs each get their own pane; extra tabs join the largest pane. Active tab of each pane = its first. */
export function tileAll(n: LayoutNode, newId: () => string): LayoutNode {
  let out: LayoutNode = emptyLayout(newId);
  for (const t of allTabs(n)) out = addTabTiled(out, t, newId);
  return mapPanes(out, (p) => ({ ...p, active: 0 }));
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function parseTab(raw: unknown): TabRef | undefined {
  if (!isRecord(raw)) return undefined;
  if (raw.kind === 'session' && typeof raw.deck_id === 'string' && raw.deck_id !== '') {
    return { kind: 'session', deck_id: raw.deck_id };
  }
  if (raw.kind === 'changes' && typeof raw.project_id === 'string' && raw.project_id !== '') {
    return { kind: 'changes', project_id: raw.project_id };
  }
  return undefined;
}

interface NormCtx {
  seenTabs: Set<string>;
  seenIds: Set<string>;
  newId: () => string;
}

function normNode(raw: unknown, ctx: NormCtx): LayoutNode | undefined {
  if (!isRecord(raw)) return undefined;
  if (raw.type === 'pane') {
    const tabs: TabRef[] = [];
    if (Array.isArray(raw.tabs)) {
      for (const rt of raw.tabs as unknown[]) {
        const t = parseTab(rt);
        if (!t) continue;
        const k = tabKey(t);
        if (ctx.seenTabs.has(k)) continue;
        ctx.seenTabs.add(k);
        tabs.push(t);
      }
    }
    let id = typeof raw.id === 'string' && raw.id !== '' ? raw.id : '';
    if (id === '' || ctx.seenIds.has(id)) id = ctx.newId();
    ctx.seenIds.add(id);
    const rawActive = typeof raw.active === 'number' && Number.isFinite(raw.active) ? Math.floor(raw.active) : 0;
    return { type: 'pane', id, tabs, active: clamp(rawActive, 0, Math.max(0, tabs.length - 1)) };
  }
  if (raw.type === 'split') {
    if (raw.dir !== 'row' && raw.dir !== 'column') return undefined;
    const ratio =
      typeof raw.ratio === 'number' && Number.isFinite(raw.ratio) ? clamp(raw.ratio, MIN_RATIO, MAX_RATIO) : 0.5;
    const a = normNode(raw.a, ctx);
    const b = normNode(raw.b, ctx);
    if (a && b) return { type: 'split', dir: raw.dir, ratio, a, b };
    return a ?? b;
  }
  return undefined;
}

/** Drops every pane past the first `limit`, collecting their tabs. */
function capPanes(n: LayoutNode, limit: number, state: { seen: number; spill: TabRef[] }): LayoutNode | undefined {
  if (n.type === 'pane') {
    if (state.seen < limit) {
      state.seen++;
      return n;
    }
    state.spill.push(...n.tabs);
    return undefined;
  }
  const a = capPanes(n.a, limit, state);
  const b = capPanes(n.b, limit, state);
  if (a && b) return { ...n, a, b };
  return a ?? b;
}

export function normalize(raw: unknown, newId: () => string): LayoutNode {
  const ctx: NormCtx = { seenTabs: new Set(), seenIds: new Set(), newId };
  const tree = normNode(raw, ctx);
  if (!tree) return emptyLayout(newId);
  if (panes(tree).length <= MAX_PANES) return tree;
  const state = { seen: 0, spill: [] as TabRef[] };
  const capped = capPanes(tree, MAX_PANES, state) ?? emptyLayout(newId);
  const kept = panes(capped);
  const lastId = kept[kept.length - 1]?.id;
  return mapPanes(capped, (p) => (p.id === lastId ? { ...p, tabs: [...p.tabs, ...state.spill] } : p));
}
