import { describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import LayoutView from './LayoutView.vue';
import PaneView from './PaneView.vue';
import { buildDeck, mountWithDeck, pane, sess, sTab, split } from './paneTestKit';

vi.mock('./TerminalView.vue', () => ({
  default: { name: 'TerminalView', props: ['target'], template: '<div data-testid="term">{{ target.deck_id }}</div>' },
}));
vi.mock('./ChangesTab.vue', () => ({
  default: { name: 'ChangesTab', props: ['projectId'], template: '<div data-testid="changes">{{ projectId }}</div>' },
}));

describe('PaneView', () => {
  it('renders a terminal for an awake session', () => {
    const lay = pane('pa', [sTab('a')]);
    const { deck } = buildDeck({ sessions: [sess('a', 'p1', 'working')], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, PaneView, { pane: lay });
    expect(w.get('[data-testid="term"]').text()).toBe('a');
    expect(w.find('[data-testid="asleep-panel"]').exists()).toBe(false);
    w.unmount();
  });

  it('renders the AsleepPanel (no terminal) for an asleep session, and Wake wakes it', async () => {
    const lay = pane('pa', [sTab('a')]);
    const { t, deck } = buildDeck({ sessions: [sess('a', 'p1', 'asleep', 'Alpha')], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, PaneView, { pane: lay });
    expect(w.find('[data-testid="term"]').exists()).toBe(false);
    await w.get('[data-testid="wake"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'wake', deck_id: 'a' }]);
    w.unmount();
  });

  it('renders ChangesTab for a changes tab', () => {
    const lay = pane('pa', [{ kind: 'changes', project_id: 'p1' }]);
    const { deck } = buildDeck({ sessions: [], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, PaneView, { pane: lay });
    expect(w.get('[data-testid="changes"]').text()).toBe('p1');
    w.unmount();
  });

  it('pressing inside a pane focuses it; only the focused pane has the accent line', async () => {
    const lay = split('row', pane('pa', [sTab('a')]), pane('pb', [sTab('b')]));
    const { deck } = buildDeck({ sessions: [sess('a', 'p1'), sess('b', 'p1')], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, LayoutView, { node: deck.currentLayout.value, path: [] });
    expect(deck.focusedPaneId.value).toBe('pa');
    expect(w.findAll('[data-testid="focus-line"]')).toHaveLength(1);
    await w.get('[data-pane-id="pb"] [data-testid="term"]').trigger('mousedown');
    await nextTick();
    expect(deck.focusedPaneId.value).toBe('pb');
    expect(w.get('[data-pane-id="pb"]').find('[data-testid="focus-line"]').exists()).toBe(true);
    expect(w.get('[data-pane-id="pa"]').find('[data-testid="focus-line"]').exists()).toBe(false);
    w.unmount();
  });

  it('shows an empty hint for a pane without tabs', () => {
    const lay = pane('pa', []);
    const { deck } = buildDeck({ sessions: [], projectLayouts: { p1: lay } });
    const w = mountWithDeck(deck, PaneView, { pane: lay });
    expect(w.get('[data-testid="pane-empty"]').text()).toBe('No open tabs');
    w.unmount();
  });
});
