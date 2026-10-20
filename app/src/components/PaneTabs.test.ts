import { afterEach, describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import { panes } from '../layout';
import { TAB_MIME } from '../useDeckDrag';
import PaneTabs from './PaneTabs.vue';
import { buildDeck, dragEvent, mountWithDeck, pane, sess, sTab } from './paneTestKit';

const projLayout = pane('pa', [sTab('a'), sTab('b'), { kind: 'changes', project_id: 'p1' }]);

function project(focused = true) {
  const kit = buildDeck({
    sessions: [sess('a', 'p1', 'idle', 'Alpha'), sess('b', 'p1', 'asleep', 'Bravo')],
    projectLayouts: { p1: projLayout },
  });
  const w = mountWithDeck(kit.deck, PaneTabs, { pane: projLayout, focused });
  return { ...kit, w };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('PaneTabs', () => {
  it('renders the tabs with Changes and marks the active one', () => {
    const { w } = project();
    expect(w.findAll('[role="tab"]').map((x) => x.text())).toEqual(['Alpha', 'Bravo', 'Changes']);
    expect(w.get('[data-testid="tab-a"]').classes()).toContain('bg-deck-term');
    expect(w.get('[data-testid="tab-b"]').classes()).not.toContain('bg-deck-term');
    expect(w.find('[data-testid="tab-project"]').exists()).toBe(false);
    w.unmount();
  });

  it('shows the accent line only when the pane is focused; tabs never carry their own accent bar', () => {
    const f = project(true);
    expect(f.w.find('[data-testid="focus-line"]').exists()).toBe(true);
    f.w.unmount();
    const u = project(false);
    expect(u.w.find('[data-testid="focus-line"]').exists()).toBe(false);
    expect(u.w.get('[data-testid="tab-a"]').classes()).toContain('bg-deck-term');
    expect(u.w.html()).not.toContain('bg-deck-accent');
    u.w.unmount();
  });

  it('Needs you tab is amber', () => {
    const lay = pane('pa', [sTab('a')]);
    const { deck } = buildDeck({ sessions: [sess('a', 'p1', 'needs_you')], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, PaneTabs, { pane: lay, focused: true });
    const tab = w.get('[data-testid="tab-a"]');
    expect(tab.classes()).toContain('bg-needs-soft');
    expect(tab.classes()).toContain('text-dot-needs');
    expect(tab.text()).toContain('Needs you');
    w.unmount();
  });

  it('dragstart sets the tab JSON and the plain name', () => {
    const { w } = project();
    const { store, dt } = dragEvent(w.get('[data-testid="tab-a"]').element, 'dragstart');
    expect(JSON.parse(store[TAB_MIME] ?? '')).toEqual({ kind: 'session', deck_id: 'a' });
    expect(store['text/plain']).toBe('Alpha');
    expect(dt.effectAllowed).toBe('move');
    w.unmount();
  });

  it('clicking a tab activates it without waking an asleep session', async () => {
    const { t, deck, w } = project();
    await w.get('[data-testid="tab-b"] [role="tab"]').trigger('click');
    expect(panes(deck.currentLayout.value)[0]?.active).toBe(1);
    expect(t.sent.some((r) => r.type === 'wake')).toBe(false);
    w.unmount();
  });

  it('the x button closes the tab; the session keeps running', async () => {
    const { t, deck, w } = project();
    await w.get('[aria-label="Close tab Alpha"]').trigger('click');
    expect(deck.openTabs['p1']).toEqual(['b']);
    expect(t.sent.some((r) => r.type === 'close_session' || r.type === 'sleep')).toBe(false);
    w.unmount();
  });

  it('plus starts a new session in a project view', async () => {
    const { t, w } = project();
    await w.get('[data-testid="new-session"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'new_session', project_id: 'p1' }]);
    w.unmount();
  });

  it('in a group view shows project names faint and hides the plus button', () => {
    const lay = pane('pg', [sTab('a'), sTab('x')]);
    const { deck } = buildDeck({
      sessions: [sess('a', 'p1', 'idle', 'Alpha'), sess('x', 'p2', 'idle', 'Xray')],
      groups: [{ id: 'g1', name: 'G', layout: lay }],
      select: { kind: 'group', id: 'g1' },
    });
    const w = mountWithDeck(deck, PaneTabs, { pane: lay, focused: true });
    const names = w.findAll('[data-testid="tab-project"]');
    expect(names.map((n) => n.text())).toEqual(['alpha', 'beta']);
    expect(names[0]?.classes()).toContain('text-deck-faint');
    expect(names[0]?.classes()).toContain('text-[11px]');
    expect(w.find('[data-testid="new-session"]').exists()).toBe(false);
    w.unmount();
  });

  it('context menu splits right using the clicked tab', async () => {
    const { deck, w } = project();
    await w.get('[data-testid="tab-b"]').trigger('contextmenu', { clientX: 20, clientY: 20 });
    await nextTick();
    const items = Array.from(document.body.querySelectorAll<HTMLElement>('[role="menuitem"]'));
    expect(items.map((i) => i.textContent?.trim())).toEqual(['Split right', 'Split down', 'Close tab']);
    items[0]?.click();
    await nextTick();
    const ps = panes(deck.currentLayout.value);
    expect(ps).toHaveLength(2);
    expect(ps.some((p) => p.tabs.length === 1 && p.tabs[0]?.kind === 'session' && p.tabs[0].deck_id === 'b')).toBe(true);
    w.unmount();
  });

  it('split is disabled with a single tab', async () => {
    const lay = pane('pa', [sTab('a')]);
    const { deck } = buildDeck({ sessions: [sess('a', 'p1')], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, PaneTabs, { pane: lay, focused: true });
    await w.get('[data-testid="tab-a"]').trigger('contextmenu', { clientX: 5, clientY: 5 });
    await nextTick();
    const first = document.body.querySelector<HTMLElement>('[role="menuitem"]');
    expect(first?.hasAttribute('disabled')).toBe(true);
    w.unmount();
  });
});
