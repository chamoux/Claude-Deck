import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, nextTick } from 'vue';
import { allTabs } from '../layout';
import type { Group, SessionState, SessionView, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import type { Deck } from '../useDeck';
import ProjectTree from './ProjectTree.vue';

const open = vi.fn();
const openPath = vi.fn();
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: (...a: unknown[]) => open(...a) }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: (...a: unknown[]) => openPath(...a) }));

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };
const sess = (deck_id: string, project_id: string, state: SessionState): SessionView => ({
  deck_id, project_id, name: deck_id, state, background_work: false, error: null,
});

function mountTree(deck: Deck) {
  const Host = defineComponent({
    emits: ['resume-past'],
    setup(_, { emit }) {
      provideDeck(deck);
      return () => h(ProjectTree, { onResumePast: (id: string) => emit('resume-past', id) });
    },
  });
  return mount(Host, { attachTo: document.body });
}
type W = ReturnType<typeof mountTree>;

function build(opts: { missing?: string[]; groups?: Group[] } | string[] = {}) {
  const { missing = [], groups = [] } = Array.isArray(opts) ? { missing: opts } : opts;
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({
    type: 'state',
    state: {
      projects: [
        { id: 'p1', path: 'C:/code/p1', name: 'alpha', shell_panel: { open: false, width: 480 } },
        { id: 'p2', path: 'C:/code/p2', name: 'beta', shell_panel: { open: false, width: 480 } },
      ],
      sessions: [
        sess('w', 'p1', 'working'),
        sess('i', 'p1', 'idle'),
        sess('n', 'p1', 'needs_you'),
        sess('a', 'p2', 'asleep'),
      ],
      shells: [],
      settings,
      missing_folders: missing,
      groups,
    },
  });
  t.sent.length = 0;
  return { t, deck, w: mountTree(deck) };
}

const menuItem = (w: W, label: string) => w.findAll('[role="menuitem"]').find((i) => i.text() === label)!;

beforeEach(() => {
  document.body.innerHTML = '';
  open.mockReset();
  openPath.mockReset();
});

describe('ProjectTree', () => {
  it('shows projects with session counts and a Needs-you badge', () => {
    const { w } = build();
    expect(w.text()).toContain('PROJECTS');
    const alpha = w.get('[data-testid="project-p1"]');
    expect(alpha.text()).toContain('alpha');
    expect(alpha.get('[data-testid="count"]').text()).toBe('3');
    expect(alpha.get('[data-testid="needs-badge"]').text()).toBe('1');
    expect(w.get('[data-testid="project-p2"]').find('[data-testid="needs-badge"]').exists()).toBe(false);
  });

  it('renders a dot class for every state', () => {
    const { w } = build();
    const cls = w.findAll('[data-testid="dot"]').map((d) => d.classes().join(' '));
    expect(cls.some((c) => c.includes('bg-dot-working'))).toBe(true);
    expect(cls.some((c) => c.includes('bg-dot-idle'))).toBe(true);
    expect(cls.some((c) => c.includes('bg-dot-needs'))).toBe(true);
    expect(cls.some((c) => c.includes('border-dot-asleep'))).toBe(true);
  });

  it('Needs you row has the label and amber tint', () => {
    const { w } = build();
    const row = w.get('[data-testid="session-n"] [data-nav]');
    expect(row.text()).toContain('Needs you');
    expect(row.classes()).toContain('bg-needs-soft');
  });

  it('collapse hides sessions but keeps the badge', async () => {
    const { w, deck } = build();
    await w.get('[data-testid="project-p1"] [data-nav]').trigger('click');
    expect(deck.selectedProjectId.value).toBe('p1');
    expect(w.find('[data-testid="session-w"]').exists()).toBe(false);
    expect(w.get('[data-testid="project-p1"] [data-testid="needs-badge"]').text()).toBe('1');
  });

  it('clicking an asleep session sends Wake', async () => {
    const { w, t } = build();
    await w.get('[data-testid="session-a"] [data-nav]').trigger('click');
    expect(t.sent).toContainEqual({ type: 'wake', deck_id: 'a' });
  });

  it('shows Folder missing and dims the project', () => {
    const { w } = build(['p2']);
    const p2 = w.get('[data-testid="project-p2"]');
    expect(p2.text()).toContain('Folder missing');
    expect(p2.get('[data-nav]').classes()).toContain('opacity-50');
  });

  it('project menu: New Claude session sends NewSession', async () => {
    const { w, t } = build();
    await w.get('[data-testid="project-p1"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'New Claude session').trigger('click');
    expect(t.sent).toContainEqual({ type: 'new_session', project_id: 'p1' });
    expect(w.find('[role="menu"]').exists()).toBe(false);
  });

  it('project menu: Resume past emits resume-past', async () => {
    const { w } = build();
    await w.get('[data-testid="project-p1"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Resume past session…').trigger('click');
    expect(w.emitted('resume-past')![0]).toEqual(['p1']);
  });

  it('project menu: Open in File Explorer calls openPath', async () => {
    const { w } = build();
    await w.get('[data-testid="project-p1"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Open in File Explorer').trigger('click');
    await flushPromises();
    expect(openPath).toHaveBeenCalledWith('C:/code/p1');
  });

  it('project menu: Remove from list calls removeProject', async () => {
    const { w, t } = build();
    await w.get('[data-testid="project-p2"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Remove from list').trigger('click');
    expect(t.sent).toContainEqual({ type: 'remove_project', project_id: 'p2', force: false });
  });

  it('session menu: Rename sends Rename', async () => {
    const { w, t } = build();
    await w.get('[data-testid="session-i"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Rename').trigger('click');
    const input = w.get('[data-testid="session-i"] input');
    await input.setValue('Renamed');
    await input.trigger('keydown', { key: 'Enter' });
    expect(t.sent).toContainEqual({ type: 'rename', deck_id: 'i', name: 'Renamed' });
    expect(w.find('[data-testid="session-i"] input').exists()).toBe(false);
  });

  it('session menu: Sleep now on a working session sends Sleep without force', async () => {
    const { w, t } = build();
    await w.get('[data-testid="session-w"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Sleep now').trigger('click');
    expect(t.sent).toContainEqual({ type: 'sleep', deck_id: 'w', force: false });
  });

  it('session menu: Sleep now is disabled when asleep', async () => {
    const { w } = build();
    await w.get('[data-testid="session-a"] [data-nav]').trigger('contextmenu');
    expect(menuItem(w, 'Sleep now').attributes('disabled')).toBeDefined();
  });

  it('session menu: Close session sends CloseSession', async () => {
    const { w, t } = build();
    await w.get('[data-testid="session-i"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Close session').trigger('click');
    expect(t.sent).toContainEqual({ type: 'close_session', deck_id: 'i', force: false });
  });

  it('+ Add opens the folder picker and adds the project', async () => {
    open.mockResolvedValue('C:/code/new');
    const { w, t } = build();
    await w.get('[data-testid="add-project"]').trigger('click');
    await flushPromises();
    expect(open).toHaveBeenCalledWith({ directory: true });
    expect(t.sent).toContainEqual({ type: 'add_project', path: 'C:/code/new' });
  });

  it('+ Add does nothing when the picker is cancelled', async () => {
    open.mockResolvedValue(null);
    const { w, t } = build();
    await w.get('[data-testid="add-project"]').trigger('click');
    await flushPromises();
    expect(t.sent).toEqual([]);
  });

  it('arrow keys move focus between rows', async () => {
    const { w } = build();
    const navs = w.findAll('[data-nav]').map((n) => n.element as HTMLElement);
    navs[0].focus();
    await w.get('nav').trigger('keydown', { key: 'ArrowDown' });
    expect(document.activeElement).toBe(navs[1]);
    await w.get('nav').trigger('keydown', { key: 'ArrowUp' });
    expect(document.activeElement).toBe(navs[0]);
  });

  it('mounts the Groups section above Projects', () => {
    const { w } = build();
    const text = w.text();
    expect(text.indexOf('GROUPS')).toBeGreaterThan(-1);
    expect(text.indexOf('GROUPS')).toBeLessThan(text.indexOf('PROJECTS'));
  });

  it('session menu: Add to group lists groups and New group…', async () => {
    const { w, t, deck } = build({ groups: [{ id: 'g1', name: 'Billing', layout: {} }] });
    await w.get('[data-testid="session-i"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Add to group').trigger('click');
    expect(w.findAll('[data-submenu] [role="menuitem"]').map((e) => e.text())).toEqual(['Billing', 'New group…']);
    await w.findAll('[data-submenu] [role="menuitem"]')[0].trigger('click');
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g1' }))).toEqual([{ kind: 'session', deck_id: 'i' }]);
    expect(t.sent).toEqual([]);
  });

  it('session menu: New group… creates a group', async () => {
    const { w, t } = build();
    await w.get('[data-testid="session-i"] [data-nav]').trigger('contextmenu');
    await menuItem(w, 'Add to group').trigger('click');
    expect(w.findAll('[data-submenu] [role="menuitem"]').map((e) => e.text())).toEqual(['New group…']);
    await w.get('[data-submenu] [role="menuitem"]').trigger('click');
    expect(t.sent).toContainEqual({ type: 'add_group', name: '' });
  });

  it('a project row is active only when that project is selected', async () => {
    const { w, deck } = build({ groups: [{ id: 'g1', name: 'Billing', layout: {} }] });
    deck.selectProject('p1');
    await nextTick();
    const row = () => w.get('[data-testid="project-p1"] > [data-nav]');
    expect(row().classes()).toContain('text-deck-text');
    deck.selectGroup('g1');
    await nextTick();
    expect(row().classes()).toContain('text-deck-muted');
  });

  it('the focused pane active session is the active row', async () => {
    const { w, deck } = build();
    deck.selectProject('p1');
    deck.openSession('i');
    await nextTick();
    expect(w.get('[data-testid="session-i"] [data-nav]').classes()).toContain('bg-deck-active');
    expect(w.get('[data-testid="session-w"] [data-nav]').classes()).not.toContain('bg-deck-active');
  });
});
