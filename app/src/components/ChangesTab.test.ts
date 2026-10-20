import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { defineComponent, h } from 'vue';
import type { Change, GitResult, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import ChangesTab from './ChangesTab.vue';

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };

function setup() {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({
    type: 'state',
    state: {
      projects: [{ id: 'p1', path: 'C:/code/shop', name: 'shop', shell_panel: { open: false, width: 480 } }],
      sessions: [],
      shells: [],
      settings,
      missing_folders: [],
    },
  });
  t.sent.length = 0;
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(ChangesTab, { projectId: 'p1' });
    },
  });
  return { t, deck, mount: () => mount(Host) };
}

const ch = (kind: Change['kind'], path: string, extra: Partial<Change> = {}): Change => ({
  kind,
  path,
  old_path: null,
  conflict: false,
  ...extra,
});
const ok = (items: Change[], total = items.length): GitResult => ({ status: 'ok', list: { items, total } });

describe('ChangesTab', () => {
  it('requests changes on mount when missing', () => {
    const { t, mount: m } = setup();
    m();
    expect(t.sent).toEqual([{ type: 'git_status', project_id: 'p1' }]);
  });

  it('does not request when already loaded', () => {
    const { t, mount: m } = setup();
    t.emit({ type: 'git_status', project_id: 'p1', result: ok([]) });
    t.sent.length = 0;
    m();
    expect(t.sent).toEqual([]);
  });

  it('shows Loading… while missing', () => {
    const { mount: m } = setup();
    expect(m().text()).toContain('Loading…');
  });

  it('groups in order with counts', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({
      type: 'git_status',
      project_id: 'p1',
      result: ok([
        ch('renamed', 'src/new.ts', { old_path: 'src/old.ts' }),
        ch('deleted', 'gone.txt'),
        ch('modified', 'a/b.ts'),
        ch('modified', 'c.ts'),
        ch('created', 'n.ts'),
      ]),
    });
    await w.vm.$nextTick();
    const heads = w.findAll('h3').map((x) => x.text().replace(/\s+/g, ' '));
    expect(heads).toEqual(['Created 1', 'Modified 2', 'Deleted 1', 'Renamed 1']);
    expect(w.find('header').text()).toContain('Changes');
    expect(w.find('header').text()).toContain('shop');
    expect(w.find('header').text()).toContain('5');
  });

  it('hides empty groups', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result: ok([ch('modified', 'x.ts')]) });
    await w.vm.$nextTick();
    expect(w.findAll('h3')).toHaveLength(1);
  });

  it('shows badge letters, file name and folder', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({
      type: 'git_status',
      project_id: 'p1',
      result: ok([ch('created', 'a/b/new.ts'), ch('modified', 'm.ts'), ch('deleted', 'd.ts'), ch('renamed', 'r2.ts', { old_path: 'r1.ts' })]),
    });
    await w.vm.$nextTick();
    expect(w.findAll('[data-testid="badge"]').map((b) => b.text())).toEqual(['C', 'M', 'D', 'R']);
    const first = w.findAll('li')[0]!;
    expect(first.text()).toContain('new.ts');
    expect(first.text()).toContain('a/b');
  });

  it('shows old → new for renames', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result: ok([ch('renamed', 'src/new.ts', { old_path: 'src/old.ts' })]) });
    await w.vm.$nextTick();
    const li = w.find('li');
    expect(li.text()).toContain('src/old.ts');
    expect(li.text()).toContain('src/new.ts');
    expect(li.find('svg').exists()).toBe(true);
  });

  it('shows the conflict tag only on conflicts', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result: ok([ch('modified', 'a.ts', { conflict: true }), ch('modified', 'b.ts')]) });
    await w.vm.$nextTick();
    const lis = w.findAll('li');
    expect(lis[0]!.text()).toContain('conflict');
    expect(lis[1]!.text()).not.toContain('conflict');
  });

  it('shows "and N more" when capped', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result: ok([ch('created', 'a.ts')], 501) });
    await w.vm.$nextTick();
    expect(w.text()).toContain('and 500 more');
  });

  it.each([
    [{ status: 'not_git' }, 'Not a git project'],
    [{ status: 'git_missing' }, "Git isn't installed"],
    [{ status: 'error', message: 'boom failed' }, 'boom failed'],
    [{ status: 'ok', list: { items: [], total: 0 } }, 'No uncommitted changes'],
  ] as [GitResult, string][])('state %j', async (result, text) => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result });
    await w.vm.$nextTick();
    expect(w.text()).toContain(text);
    expect(w.findAll('li')).toHaveLength(0);
  });

  it('error message is red', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result: { status: 'error', message: 'bad' } });
    await w.vm.$nextTick();
    expect(w.find('[role="alert"]').classes().join(' ')).toContain('text-deck-danger');
  });

  it('refresh button sends GitStatus', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.sent.length = 0;
    await w.find('button[aria-label="Refresh changes"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'git_status', project_id: 'p1' }]);
  });

  it('lines are not buttons', async () => {
    const { t, mount: m } = setup();
    const w = m();
    t.emit({ type: 'git_status', project_id: 'p1', result: ok([ch('created', 'a.ts')]) });
    await w.vm.$nextTick();
    expect(w.find('li button').exists()).toBe(false);
  });
});
