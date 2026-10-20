import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it } from 'vitest';
import { defineComponent, h } from 'vue';
import { allTabs, panes } from '../layout';
import type { Group, SessionState, SessionView, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import GroupList from './GroupList.vue';

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };
const sess = (deck_id: string, state: SessionState): SessionView => ({
  deck_id, project_id: 'p1', name: deck_id, state, background_work: false, error: null,
});
const tab = (deck_id: string) => ({ kind: 'session' as const, deck_id });
const groups: Group[] = [
  {
    id: 'g1',
    name: 'Billing',
    layout: { type: 'pane', id: 'x', tabs: [tab('a'), tab('b'), tab('n'), { kind: 'changes', project_id: 'p1' }], active: 0 },
  },
  { id: 'g2', name: 'Empty one', layout: {} },
];

function build(gs: Group[] = groups) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({
    type: 'state',
    state: {
      projects: [{ id: 'p1', path: 'C:/p1', name: 'alpha', shell_panel: { open: false, width: 480 } }],
      sessions: [sess('a', 'idle'), sess('b', 'working'), sess('n', 'needs_you'), sess('c', 'idle')],
      shells: [],
      settings,
      missing_folders: [],
      groups: gs,
    },
  });
  t.sent.length = 0;
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(GroupList);
    },
  });
  return { t, deck, w: mount(Host, { attachTo: document.body }) };
}
type W = ReturnType<typeof build>['w'];

const dt = (types: string[], data: Record<string, string> = {}) => ({
  types,
  getData: (k: string) => data[k] ?? '',
  dropEffect: '',
});
const menuItem = (w: W, label: string) => w.findAll('[role="menuitem"]').find((i) => i.text() === label)!;

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('GroupList', () => {
  it('shows the label, rows with a session count and the Needs-you pill', () => {
    const { w } = build();
    expect(w.text()).toContain('GROUPS');
    const row = w.get('[data-testid="group-g1"]');
    expect(row.text()).toContain('Billing');
    expect(row.get('[data-testid="group-count"]').text()).toBe('3');
    expect(row.get('[data-testid="group-needs"]').text()).toBe('1');
    expect(row.get('[data-nav]').classes()).toContain('h-[30px]');
    const empty = w.get('[data-testid="group-g2"]');
    expect(empty.get('[data-testid="group-count"]').text()).toBe('0');
    expect(empty.find('[data-testid="group-needs"]').exists()).toBe(false);
  });

  it('empty groups section shows the header, + and one helper line', () => {
    const { w } = build([]);
    expect(w.text()).toContain('GROUPS');
    expect(w.text()).toContain('Combine sessions from any projects');
    expect(w.find('[data-testid="new-group"]').exists()).toBe(true);
    expect(w.findAll('[data-nav]')).toHaveLength(0);
  });

  it('+ creates a group', async () => {
    const { w, t } = build();
    const btn = w.get('[data-testid="new-group"]');
    expect(btn.attributes('aria-label')).toBe('New group');
    expect(btn.attributes('title')).toBe('New group');
    await btn.trigger('click');
    expect(t.sent).toContainEqual({ type: 'add_group', name: '' });
  });

  it('clicking a row selects the group and marks it active', async () => {
    const { w, deck } = build();
    await w.get('[data-testid="group-g1"] [data-nav]').trigger('click');
    expect(deck.selected.value).toEqual({ kind: 'group', id: 'g1' });
    expect(w.get('[data-testid="group-g1"] [data-nav]').classes()).toContain('bg-deck-active');
    expect(w.get('[data-testid="group-g1"] [data-testid="bar"]').classes()).toContain('bg-deck-accent');
    expect(w.get('[data-testid="group-g2"] [data-nav]').classes()).not.toContain('bg-deck-active');
  });

  it('double-click renames inline: Enter saves', async () => {
    const { w, t } = build();
    await w.get('[data-testid="group-g1"] [data-nav]').trigger('dblclick');
    const input = w.get('[data-testid="group-g1"] input');
    await input.setValue('Payments');
    await input.trigger('keydown', { key: 'Enter' });
    expect(t.sent).toContainEqual({ type: 'rename_group', group_id: 'g1', name: 'Payments' });
    expect(w.find('[data-testid="group-g1"] input').exists()).toBe(false);
  });

  it('rename: Escape cancels without sending', async () => {
    const { w, t } = build();
    await w.get('[data-testid="group-g1"] [data-nav]').trigger('dblclick');
    const input = w.get('[data-testid="group-g1"] input');
    await input.setValue('Nope');
    await input.trigger('keydown', { key: 'Escape' });
    expect(t.sent).toEqual([]);
    expect(w.find('[data-testid="group-g1"] input').exists()).toBe(false);
  });

  it('menu: Rename starts inline rename', async () => {
    const { w } = build();
    await w.get('[data-testid="group-g1"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Rename').trigger('click');
    expect(w.find('[data-testid="group-g1"] input').exists()).toBe(true);
  });

  it('menu: Arrange side by side sits above Rename and tiles the group', async () => {
    const { w, deck } = build();
    await w.get('[data-testid="group-g1"] [data-nav]').trigger('contextmenu');
    const labels = w.findAll('[role="menuitem"]').map((i) => i.text());
    expect(labels.indexOf('Arrange side by side')).toBeGreaterThanOrEqual(0);
    expect(labels.indexOf('Arrange side by side')).toBeLessThan(labels.indexOf('Rename'));
    expect(menuItem(w, 'Arrange side by side').find('svg').exists()).toBe(true);
    await menuItem(w, 'Arrange side by side').trigger('click');
    expect(panes(deck.layoutOf({ kind: 'group', id: 'g1' }))).toHaveLength(4);
  });

  it('menu: Delete group goes through the confirm', async () => {
    const { w, t, deck } = build();
    await w.get('[data-testid="group-g1"] [data-nav]').trigger('contextmenu');
    const del = menuItem(w, 'Delete group');
    expect(del.classes().join(' ')).toContain('text-deck-danger');
    await del.trigger('click');
    expect(t.sent).toEqual([]);
    expect(deck.confirm.value?.message).toContain('Billing');
    deck.confirmYes();
    expect(t.sent).toContainEqual({ type: 'remove_group', group_id: 'g1' });
  });

  it('highlights while a session or tab drag is over the row', async () => {
    const { w } = build();
    const row = w.get('[data-testid="group-g2"] [data-nav]');
    await row.trigger('dragover', { dataTransfer: dt(['application/x-deck-session']) });
    expect(row.classes()).toContain('bg-deck-active');
    expect(row.classes().join(' ')).toContain('outline-dashed');
    await row.trigger('dragleave', { dataTransfer: dt(['application/x-deck-session']) });
    expect(row.classes().join(' ')).not.toContain('outline-dashed');
    await row.trigger('dragover', { dataTransfer: dt(['application/x-deck-tab']) });
    expect(row.classes().join(' ')).toContain('outline-dashed');
  });

  it('ignores unrelated drags', async () => {
    const { w } = build();
    const row = w.get('[data-testid="group-g2"] [data-nav]');
    await row.trigger('dragover', { dataTransfer: dt(['text/plain']) });
    expect(row.classes().join(' ')).not.toContain('outline-dashed');
  });

  it('dropping a sidebar session adds it to the group', async () => {
    const { w, deck } = build();
    const row = w.get('[data-testid="group-g2"] [data-nav]');
    await row.trigger('dragover', { dataTransfer: dt(['application/x-deck-session']) });
    await row.trigger('drop', {
      dataTransfer: dt(['application/x-deck-session'], { 'application/x-deck-session': '{"deck_id":"c"}' }),
    });
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g2' }))).toEqual([tab('c')]);
    expect(row.classes().join(' ')).not.toContain('outline-dashed');
  });

  it('dropping a session tab adds it; a changes tab is ignored', async () => {
    const { w, deck } = build();
    const row = w.get('[data-testid="group-g2"] [data-nav]');
    await row.trigger('drop', {
      dataTransfer: dt(['application/x-deck-tab'], {
        'application/x-deck-tab': JSON.stringify({ kind: 'changes', project_id: 'p1' }),
      }),
    });
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g2' }))).toEqual([]);
    await row.trigger('drop', {
      dataTransfer: dt(['application/x-deck-tab'], { 'application/x-deck-tab': JSON.stringify(tab('b')) }),
    });
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g2' }))).toEqual([tab('b')]);
  });
});
