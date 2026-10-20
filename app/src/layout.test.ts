import { describe, expect, it } from 'vitest';
import {
  MAX_PANES,
  MAX_RATIO,
  MIN_RATIO,
  addTab,
  addTabTiled,
  allTabs,
  canDrop,
  closeTab,
  dropTab,
  emptyLayout,
  findPane,
  findTab,
  neighbour,
  normalize,
  panes,
  prune,
  setActive,
  setRatio,
  splitPane,
  tabKey,
  tileAll,
  type LayoutNode,
  type Pane,
  type Split,
  type TabRef,
} from './layout';

function counter(prefix = 'n'): () => string {
  let i = 0;
  return () => `${prefix}${++i}`;
}
const s = (id: string): TabRef => ({ kind: 'session', deck_id: id });
const ch = (id: string): TabRef => ({ kind: 'changes', project_id: id });
function pane(id: string, tabs: TabRef[], active = 0): Pane {
  return { type: 'pane', id, tabs, active };
}
function split(dir: 'row' | 'column', a: LayoutNode, b: LayoutNode, ratio = 0.5): Split {
  return { type: 'split', dir, ratio, a, b };
}
function ids(n: LayoutNode): string[] {
  return panes(n).map((p) => p.id);
}
function keys(n: LayoutNode): string[] {
  return allTabs(n).map(tabKey);
}
function asSplit(n: LayoutNode): Split {
  if (n.type !== 'split') throw new Error('expected split');
  return n;
}
function asPane(n: LayoutNode): Pane {
  if (n.type !== 'pane') throw new Error('expected pane');
  return n;
}
function deepFreeze<T>(v: T): T {
  if (typeof v === 'object' && v !== null) {
    Object.freeze(v);
    for (const c of Object.values(v)) deepFreeze(c);
  }
  return v;
}

describe('basics', () => {
  it('tabKey', () => {
    expect(tabKey(s('a'))).toBe('session:a');
    expect(tabKey(ch('p'))).toBe('changes:p');
  });
  it('emptyLayout', () => {
    expect(emptyLayout(() => 'x')).toEqual({ type: 'pane', id: 'x', tabs: [], active: 0 });
  });
  it('panes in order, findPane, findTab, allTabs', () => {
    const t = split('row', pane('A', [s('1')]), split('column', pane('B', [s('2'), s('3')]), pane('C', [])));
    expect(ids(t)).toEqual(['A', 'B', 'C']);
    expect(findPane(t, 'B')?.tabs).toHaveLength(2);
    expect(findPane(t, 'Z')).toBeUndefined();
    expect(findTab(t, 'session:3')).toMatchObject({ index: 1, pane: { id: 'B' } });
    expect(findTab(t, 'session:9')).toBeUndefined();
    expect(keys(t)).toEqual(['session:1', 'session:2', 'session:3']);
  });
});

describe('dropTab', () => {
  const base = (): LayoutNode => split('row', pane('A', [s('1'), s('2')]), pane('B', [s('3')]));

  it.each([
    ['left', 'row', true],
    ['right', 'row', false],
    ['top', 'column', true],
    ['bottom', 'column', false],
  ] as const)('edge %s makes a %s split with the new pane first=%s', (zone, dir, first) => {
    const r = dropTab(base(), 'B', zone, s('1'), counter());
    const outer = asSplit(r);
    const inner = asSplit(outer.b);
    expect(inner.dir).toBe(dir);
    expect(inner.ratio).toBe(0.5);
    const fresh = asPane(first ? inner.a : inner.b);
    const old = asPane(first ? inner.b : inner.a);
    expect(fresh.tabs).toEqual([s('1')]);
    expect(fresh.id).toBe('n1');
    expect(old.id).toBe('B');
    expect(old.tabs).toEqual([s('3')]);
    expect(keys(r).sort()).toEqual(['session:1', 'session:2', 'session:3']);
    expect(findPane(r, 'A')?.tabs).toEqual([s('2')]);
  });

  it('centre appends to the target and activates', () => {
    const r = dropTab(base(), 'B', 'center', s('1'), counter());
    expect(findPane(r, 'B')).toMatchObject({ tabs: [s('3'), s('1')], active: 1 });
    expect(findPane(r, 'A')?.tabs).toEqual([s('2')]);
  });

  it('moving the last tab of A to B centre collapses A', () => {
    const t = split('row', pane('A', [s('1')]), pane('B', [s('3')]));
    const r = dropTab(t, 'B', 'center', s('1'), counter());
    expect(r.type).toBe('pane');
    expect(asPane(r).id).toBe('B');
    expect(asPane(r).tabs).toEqual([s('3'), s('1')]);
  });

  it('adopts a tab that is not in the tree yet', () => {
    const r = dropTab(base(), 'A', 'center', s('9'), counter());
    expect(findPane(r, 'A')?.tabs).toEqual([s('1'), s('2'), s('9')]);
  });

  it('returns the same node when refused', () => {
    const t = base();
    expect(dropTab(t, 'A', 'center', s('1'), counter())).toBe(t);
    expect(dropTab(t, 'nope', 'left', s('1'), counter())).toBe(t);
  });

  it('keeps the source active tab sensible', () => {
    const t = split('row', pane('A', [s('1'), s('2'), s('3')], 2), pane('B', []));
    const r = dropTab(t, 'B', 'center', s('1'), counter());
    expect(findPane(r, 'A')).toMatchObject({ tabs: [s('2'), s('3')], active: 1 });
  });
});

describe('pane cap', () => {
  it('allows a 4th pane, refuses a 5th', () => {
    const id = counter();
    let t: LayoutNode = pane('A', [s('1'), s('2'), s('3'), s('4'), s('5')]);
    t = dropTab(t, 'A', 'right', s('2'), id);
    t = dropTab(t, 'A', 'bottom', s('3'), id);
    expect(canDrop(t, 'A', 'right', s('4'))).toBe(true);
    t = dropTab(t, 'A', 'right', s('4'), id);
    expect(panes(t)).toHaveLength(MAX_PANES);
    expect(canDrop(t, 'A', 'left', s('5'))).toBe(false);
    expect(dropTab(t, 'A', 'left', s('5'), id)).toBe(t);
  });

  it('allows an edge move of a lone tab at the cap', () => {
    const t = split(
      'row',
      split('column', pane('A', [s('1'), s('9')]), pane('B', [s('2')])),
      split('column', pane('C', [s('3')]), pane('D', [s('4')])),
    );
    expect(panes(t)).toHaveLength(4);
    expect(canDrop(t, 'A', 'right', s('9'))).toBe(false);
    expect(canDrop(t, 'A', 'right', s('2'))).toBe(true);
    const r = dropTab(t, 'A', 'right', s('2'), counter());
    expect(panes(r)).toHaveLength(4);
    expect(ids(r)).not.toContain('B');
  });

  it('canDrop agrees with dropTab', () => {
    const t = split('row', pane('A', [s('1')]), pane('B', [s('2'), s('3')]));
    for (const target of ['A', 'B', 'Z']) {
      for (const zone of ['left', 'right', 'top', 'bottom', 'center'] as const) {
        for (const tab of [s('1'), s('2'), s('7')]) {
          const r = dropTab(t, target, zone, tab, counter());
          if (canDrop(t, target, zone, tab)) expect(panes(r).length).toBeLessThanOrEqual(MAX_PANES);
          else expect(r).toBe(t);
        }
      }
    }
  });

  it("centre on the tab's own pane is not allowed", () => {
    const t = pane('A', [s('1'), s('2')]);
    expect(canDrop(t, 'A', 'center', s('1'))).toBe(false);
    expect(canDrop(t, 'A', 'right', s('1'))).toBe(true);
  });
});

describe('addTab', () => {
  it('adds and activates, moving from elsewhere', () => {
    const t = split('row', pane('A', [s('1')]), pane('B', [s('2')]));
    const r = addTab(t, 'B', s('1'));
    expect(r.type).toBe('pane');
    expect(asPane(r).tabs).toEqual([s('2'), s('1')]);
    expect(asPane(r).active).toBe(1);
  });
  it('re-activates a tab already in the pane without duplicating', () => {
    const t = pane('A', [s('1'), s('2')], 1);
    expect(addTab(t, 'A', s('1'))).toEqual(pane('A', [s('1'), s('2')], 0));
  });
  it('ignores an unknown pane', () => {
    const t = pane('A', [s('1')]);
    expect(addTab(t, 'Z', s('2'))).toBe(t);
  });
});

describe('closeTab', () => {
  it('fixes active and collapses an emptied pane', () => {
    const t = split('row', pane('A', [s('1'), s('2'), s('3')], 2), pane('B', [s('4')]));
    const r1 = closeTab(t, 'session:3');
    expect(findPane(r1, 'A')).toMatchObject({ tabs: [s('1'), s('2')], active: 1 });
    const r2 = closeTab(t, 'session:4');
    expect(r2.type).toBe('pane');
    expect(asPane(r2).id).toBe('A');
  });
  it('keeps the active tab when an earlier one is closed', () => {
    const t = pane('A', [s('1'), s('2'), s('3')], 1);
    expect(asPane(closeTab(t, 'session:1'))).toMatchObject({ tabs: [s('2'), s('3')], active: 0 });
  });
  it('the last pane remains, empty', () => {
    const t = pane('A', [s('1')]);
    expect(closeTab(t, 'session:1')).toEqual(pane('A', [], 0));
  });
  it('unknown key is a no-op', () => {
    const t = pane('A', [s('1')]);
    expect(closeTab(t, 'session:zz')).toEqual(t);
  });
});

describe('setActive / setRatio', () => {
  it('setActive clamps', () => {
    const t = pane('A', [s('1'), s('2')]);
    expect(asPane(setActive(t, 'A', 1)).active).toBe(1);
    expect(asPane(setActive(t, 'A', 99)).active).toBe(1);
    expect(asPane(setActive(t, 'A', -3)).active).toBe(0);
    expect(setActive(t, 'Z', 1)).toEqual(t);
  });
  it('setRatio clamps and follows the path', () => {
    const t = split('row', pane('A', []), split('column', pane('B', []), pane('C', [])));
    expect(asSplit(setRatio(t, [], 0.3)).ratio).toBe(0.3);
    expect(asSplit(setRatio(t, [], 0.01)).ratio).toBe(MIN_RATIO);
    expect(asSplit(setRatio(t, [], 5)).ratio).toBe(MAX_RATIO);
    const r = setRatio(t, ['b'], 0.7);
    expect(asSplit(asSplit(r).b).ratio).toBe(0.7);
    expect(asSplit(r).ratio).toBe(0.5);
  });
  it('setRatio on a bad path is a no-op', () => {
    const t = split('row', pane('A', []), pane('B', []));
    expect(setRatio(t, ['a', 'a'], 0.3)).toEqual(t);
  });
});

describe('prune', () => {
  it('removes tabs and collapses', () => {
    const t = split('row', pane('A', [s('1'), s('2')]), split('column', pane('B', [s('3')]), pane('C', [s('4')])));
    const r = prune(t, (x) => x.kind === 'session' && x.deck_id !== '3' && x.deck_id !== '1');
    expect(keys(r)).toEqual(['session:2', 'session:4']);
    expect(ids(r)).toEqual(['A', 'C']);
    expect(asSplit(r).dir).toBe('row');
    expect(asSplit(r).b.type).toBe('pane');
  });
  it('keeps one empty pane if everything goes', () => {
    const r = prune(split('row', pane('A', [s('1')]), pane('B', [s('2')])), () => false);
    expect(r.type).toBe('pane');
    expect(asPane(r).tabs).toEqual([]);
  });
});

describe('splitPane', () => {
  const t = pane('A', [s('1'), s('2')], 1);
  it('moves the active tab into a new pane after it', () => {
    const r = splitPane(t, 'A', 'row', counter());
    const sp = asSplit(r);
    expect(sp.dir).toBe('row');
    expect(asPane(sp.a)).toMatchObject({ id: 'A', tabs: [s('1')] });
    expect(asPane(sp.b)).toMatchObject({ id: 'n1', tabs: [s('2')] });
    expect(asSplit(splitPane(t, 'A', 'column', counter())).dir).toBe('column');
  });
  it('is a no-op for a single tab, unknown pane, or at the cap', () => {
    const one = pane('A', [s('1')]);
    expect(splitPane(one, 'A', 'row', counter())).toBe(one);
    expect(splitPane(t, 'Z', 'row', counter())).toBe(t);
    const full = split(
      'row',
      split('column', pane('A', [s('1'), s('9')]), pane('B', [s('2')])),
      split('column', pane('C', [s('3')]), pane('D', [s('4')])),
    );
    expect(splitPane(full, 'A', 'row', counter())).toBe(full);
  });
});

describe('neighbour', () => {
  // A | B
  // -----
  // C | D
  const grid = split(
    'column',
    split('row', pane('A', []), pane('B', [])),
    split('row', pane('C', []), pane('D', [])),
  );
  it('works in a 2x2 layout', () => {
    expect(neighbour(grid, 'A', 'right')).toBe('B');
    expect(neighbour(grid, 'A', 'down')).toBe('C');
    expect(neighbour(grid, 'D', 'left')).toBe('C');
    expect(neighbour(grid, 'D', 'up')).toBe('B');
    expect(neighbour(grid, 'A', 'left')).toBeUndefined();
    expect(neighbour(grid, 'A', 'up')).toBeUndefined();
    expect(neighbour(grid, 'D', 'right')).toBeUndefined();
    expect(neighbour(grid, 'D', 'down')).toBeUndefined();
  });
  it('prefers the largest overlap', () => {
    // left column A, right column split top (small) / bottom (large)
    const t = split('row', pane('A', []), split('column', pane('B', []), pane('C', []), 0.2), 0.5);
    expect(neighbour(t, 'A', 'right')).toBe('C');
    expect(neighbour(t, 'B', 'left')).toBe('A');
  });
  it('unknown pane gives undefined', () => {
    expect(neighbour(grid, 'Z', 'left')).toBeUndefined();
  });
});

describe('normalize', () => {
  it('garbage gives an empty layout', () => {
    for (const g of [null, undefined, 5, 'x', [], {}, { type: 'nope' }, { type: 'split' }]) {
      const r = normalize(g, () => 'fresh');
      expect(r).toEqual({ type: 'pane', id: 'fresh', tabs: [], active: 0 });
    }
  });
  it('drops unknown tabs, dedupes (first wins) and fixes active', () => {
    const raw = {
      type: 'split',
      dir: 'row',
      ratio: 0.5,
      a: {
        type: 'pane',
        id: 'A',
        tabs: [{ kind: 'session', deck_id: '1' }, { kind: 'bogus' }, { kind: 'session' }, 7],
        active: 9,
      },
      b: {
        type: 'pane',
        id: 'B',
        tabs: [
          { kind: 'session', deck_id: '1' },
          { kind: 'changes', project_id: 'p' },
        ],
        active: -2,
      },
    };
    const r = normalize(raw, counter());
    expect(findPane(r, 'A')).toMatchObject({ tabs: [s('1')], active: 0 });
    expect(findPane(r, 'B')).toMatchObject({ tabs: [ch('p')], active: 0 });
  });
  it('assigns missing pane ids', () => {
    const r = normalize({ type: 'pane', tabs: [] }, () => 'gen');
    expect(asPane(r).id).toBe('gen');
  });
  it('clamps bad ratios', () => {
    const mk = (ratio: unknown) => ({ type: 'split', dir: 'column', ratio, a: pane('A', []), b: pane('B', []) });
    expect(asSplit(normalize(mk(0), counter())).ratio).toBe(MIN_RATIO);
    expect(asSplit(normalize(mk(2), counter())).ratio).toBe(MAX_RATIO);
    expect(asSplit(normalize(mk('x'), counter())).ratio).toBe(0.5);
  });
  it('collapses a split with an invalid child', () => {
    const r = normalize(
      { type: 'split', dir: 'row', ratio: 0.5, a: { type: 'wat' }, b: pane('B', [s('1')]) },
      counter(),
    );
    expect(r).toEqual(pane('B', [s('1')]));
  });
  it('caps 6 panes at 4 and keeps every tab', () => {
    let t: LayoutNode = pane('P6', [s('6')]);
    for (let i = 5; i >= 1; i--) t = split('row', pane(`P${i}`, [s(String(i))]), t);
    expect(panes(t)).toHaveLength(6);
    const r = normalize(JSON.parse(JSON.stringify(t)), counter());
    expect(ids(r)).toEqual(['P1', 'P2', 'P3', 'P4']);
    expect(findPane(r, 'P4')?.tabs).toEqual([s('4'), s('5'), s('6')]);
    expect(keys(r)).toEqual(['session:1', 'session:2', 'session:3', 'session:4', 'session:5', 'session:6']);
  });
  it('replaces duplicate pane ids', () => {
    const r = normalize(split('row', pane('A', []), pane('A', [])), counter());
    expect(new Set(ids(r)).size).toBe(2);
  });
  it('round-trips through JSON', () => {
    const t = split(
      'row',
      pane('A', [s('1'), ch('p')], 1),
      split('column', pane('B', [s('2')]), pane('C', []), 0.3),
      0.6,
    );
    expect(normalize(JSON.parse(JSON.stringify(t)), counter())).toEqual(t);
  });
});

describe('immutability', () => {
  it('no function mutates or chokes on a frozen input', () => {
    const t = split(
      'row',
      pane('A', [s('1'), s('2')], 1),
      split('column', pane('B', [s('3')]), pane('C', [ch('p')])),
    );
    const before = JSON.stringify(t);
    deepFreeze(t);
    const id = counter();
    const run: Array<() => unknown> = [
      () => panes(t),
      () => findPane(t, 'A'),
      () => findTab(t, 'session:1'),
      () => allTabs(t),
      () => canDrop(t, 'B', 'right', s('1')),
      () => dropTab(t, 'B', 'right', s('1'), id),
      () => dropTab(t, 'B', 'center', s('1'), id),
      () => dropTab(t, 'A', 'top', s('3'), id),
      () => addTab(t, 'C', s('1')),
      () => closeTab(t, 'session:3'),
      () => closeTab(t, 'session:1'),
      () => setActive(t, 'A', 0),
      () => setRatio(t, ['b'], 0.4),
      () => prune(t, () => false),
      () => splitPane(t, 'A', 'row', id),
      () => neighbour(t, 'A', 'right'),
      () => normalize(t, id),
      () => addTabTiled(t, s('9'), id),
      () => addTabTiled(t, s('1'), id),
      () => tileAll(t, id),
    ];
    for (const f of run) expect(f).not.toThrow();
    expect(JSON.stringify(t)).toBe(before);
  });
});

describe('addTabTiled / tileAll', () => {
  const grow = (count: number): LayoutNode => {
    const id = counter();
    let n: LayoutNode = emptyLayout(id);
    for (let i = 1; i <= count; i++) n = addTabTiled(n, s(String(i)), id);
    return n;
  };
  const tabsOf = (n: LayoutNode): string[][] => panes(n).map((p) => p.tabs.map(tabKey));

  it('fills an empty root pane instead of splitting', () => {
    const n = grow(1);
    expect(asPane(n).tabs).toEqual([s('1')]);
  });

  it('1 -> 2 puts the sessions side by side', () => {
    const n = asSplit(grow(2));
    expect(n.dir).toBe('row');
    expect(tabsOf(n)).toEqual([['session:1'], ['session:2']]);
  });

  it('3 gives left full + right stacked', () => {
    const n = asSplit(grow(3));
    expect(n.dir).toBe('row');
    expect(asPane(n.a).tabs).toEqual([s('1')]);
    const right = asSplit(n.b);
    expect(right.dir).toBe('column');
    expect(tabsOf(right)).toEqual([['session:2'], ['session:3']]);
  });

  it('4 gives a 2x2 grid', () => {
    const n = asSplit(grow(4));
    expect(n.dir).toBe('row');
    const left = asSplit(n.a);
    const right = asSplit(n.b);
    expect(left.dir).toBe('column');
    expect(right.dir).toBe('column');
    expect(tabsOf(n)).toEqual([['session:1'], ['session:4'], ['session:2'], ['session:3']]);
  });

  it('at the cap a 5th tab joins the largest pane (later wins ties) and becomes active', () => {
    const n = grow(5);
    expect(panes(n)).toHaveLength(MAX_PANES);
    expect(tabsOf(n)).toEqual([['session:1'], ['session:4'], ['session:2'], ['session:3', 'session:5']]);
    expect(panes(n)[3]?.active).toBe(1);
  });

  it('moves an existing tab under the same rules', () => {
    const id = counter();
    const start = pane('A', [s('1'), s('2')]);
    const n = addTabTiled(start, s('2'), id);
    expect(tabsOf(n)).toEqual([['session:1'], ['session:2']]);
    expect(asSplit(n).dir).toBe('row');
    const moved = addTabTiled(grow(3), s('1'), id);
    expect(keys(moved).sort()).toEqual(['session:1', 'session:2', 'session:3']);
    expect(panes(moved)).toHaveLength(3);
  });

  it('moving the only tab keeps a single pane', () => {
    const n = addTabTiled(pane('A', [s('1')]), s('1'), counter());
    expect(asPane(n).tabs).toEqual([s('1')]);
  });

  it('tileAll turns one pane with 3 tabs into the 3-pane shape', () => {
    const n = asSplit(tileAll(pane('A', [s('1'), s('2'), s('3')], 2), counter()));
    expect(n.dir).toBe('row');
    expect(asPane(n.a).tabs).toEqual([s('1')]);
    expect(asSplit(n.b).dir).toBe('column');
    expect(panes(n)).toHaveLength(3);
    expect(panes(n).every((p) => p.active === 0)).toBe(true);
  });

  it('tileAll on 6 tabs gives 4 panes with the extras in the largest pane', () => {
    const six = pane('A', ['1', '2', '3', '4', '5', '6'].map(s));
    const n = tileAll(six, counter());
    expect(panes(n)).toHaveLength(4);
    expect(keys(n).sort()).toEqual(['1', '2', '3', '4', '5', '6'].map((i) => `session:${i}`));
    expect(panes(n)[3]?.tabs).toEqual([s('3'), s('5'), s('6')]);
  });

  it('tileAll on an empty layout stays one empty pane', () => {
    expect(asPane(tileAll(pane('A', []), counter())).tabs).toEqual([]);
  });

  it('does not mutate a frozen input', () => {
    const t = deepFreeze(split('row', pane('A', [s('1'), s('2')]), pane('B', [s('3')])));
    const before = JSON.stringify(t);
    expect(() => addTabTiled(t, s('4'), counter())).not.toThrow();
    expect(() => tileAll(t, counter())).not.toThrow();
    expect(JSON.stringify(t)).toBe(before);
  });
});
