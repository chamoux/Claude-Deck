import { describe, expect, it } from 'vitest';
import type { Event, GitResult, Group, Project, Request } from './protocol';
import type { LayoutNode } from './layout';
import { targetKey } from './protocol';

function roundTrip<T>(json: string, value: T): void {
  expect(JSON.parse(json)).toEqual(value);
  expect(JSON.stringify(value)).toBe(json);
}

describe('protocol wire format', () => {
  it('matches the strings pinned in the Rust tests', () => {
    const attach: Request = { type: 'attach', target: { kind: 'shell', project_id: 'p' }, rows: 40, cols: 120 };
    roundTrip('{"type":"attach","target":{"kind":"shell","project_id":"p"},"rows":40,"cols":120}', attach);
    const memory: Event = { type: 'memory', awake: 2, bytes: 5 };
    roundTrip('{"type":"memory","awake":2,"bytes":5}', memory);
    const notGit: GitResult = { status: 'not_git' };
    roundTrip('{"status":"not_git"}', notGit);
    const getState: Request = { type: 'get_state' };
    roundTrip('{"type":"get_state"}', getState);
  });

  it('matches the layout and group requests pinned in the Rust tests', () => {
    const layout: LayoutNode = { type: 'pane', id: 'p1', tabs: [], active: 0 };
    const setGroup: Request = { type: 'set_group_layout', group_id: 'g', layout };
    roundTrip('{"type":"set_group_layout","group_id":"g","layout":{"type":"pane","id":"p1","tabs":[],"active":0}}', setGroup);
    const setProject: Request = { type: 'set_project_layout', project_id: 'p', layout: null };
    roundTrip('{"type":"set_project_layout","project_id":"p","layout":null}', setProject);
    roundTrip('{"type":"add_group","name":"Billing"}', { type: 'add_group', name: 'Billing' } satisfies Request);
    roundTrip('{"type":"rename_group","group_id":"g","name":"x"}', { type: 'rename_group', group_id: 'g', name: 'x' } satisfies Request);
    roundTrip('{"type":"remove_group","group_id":"g"}', { type: 'remove_group', group_id: 'g' } satisfies Request);
    roundTrip('{"type":"move_project","project_id":"p","index":1}', { type: 'move_project', project_id: 'p', index: 1 } satisfies Request);
  });

  it('accepts a new group with an empty layout and old projects without one', () => {
    const g: Group = JSON.parse('{"id":"g","name":"Billing","layout":{}}');
    expect(g.layout).toEqual({});
    const p: Project = JSON.parse('{"id":"p","path":"C:/x","name":"x","shell_panel":{"open":false,"width":480}}');
    expect(p.layout).toBeUndefined();
  });

  it('builds target keys', () => {
    expect(targetKey({ kind: 'session', deck_id: 'a' })).toBe('session:a');
    expect(targetKey({ kind: 'shell', project_id: 'p' })).toBe('shell:p');
  });
});
