import { mount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import type { SessionState, SessionView, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import CloseDialog from './CloseDialog.vue';

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };
const sess = (id: string, state: SessionState, name: string | null): SessionView => ({
  deck_id: id,
  project_id: 'p1',
  name,
  state,
  background_work: false,
  error: null,
});

function setup(sessions: SessionView[]) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({ type: 'state', state: { projects: [], sessions, shells: [], settings, missing_folders: [] } });
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(CloseDialog);
    },
  });
  const w = mount(Host, { attachTo: document.body });
  return { t, deck, w };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('CloseDialog', () => {
  it('renders nothing while closed', () => {
    const { w } = setup([sess('a', 'working', 'A')]);
    expect(w.find('[role=dialog]').exists()).toBe(false);
  });

  it('is an accessible modal with plural title and busy list', async () => {
    const { deck, w } = setup([sess('a', 'working', 'Alpha'), sess('b', 'needs_you', null), sess('c', 'idle', 'Idle one')]);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    const dlg = w.get('[role=dialog]');
    expect(dlg.attributes('aria-modal')).toBe('true');
    const labelId = dlg.attributes('aria-labelledby')!;
    expect(w.get(`#${labelId}`).text()).toBe('2 sessions are still working');
    const items = w.findAll('li');
    expect(items.map((i) => i.get('[data-name]').text())).toEqual(['Alpha', 'Untitled session']);
    expect(items[0]!.text()).not.toContain('Needs you');
    expect(items[1]!.text()).toContain('Needs you');
    expect(items[0]!.find('[data-state=working]').exists()).toBe(true);
    expect(items[1]!.find('[data-state=needs_you]').exists()).toBe(true);
  });

  it('uses singular title for one session and focuses the primary button', async () => {
    const { deck, w } = setup([sess('a', 'working', 'Alpha')]);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    await w.vm.$nextTick();
    expect(w.get('h2').text()).toBe('1 session is still working');
    expect(document.activeElement?.textContent?.trim()).toBe('Keep running in background');
  });

  it('Keep running calls close(true)', async () => {
    const { t, deck, w } = setup([sess('a', 'working', 'A')]);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    await w.findAll('button').find((b) => b.text() === 'Keep running in background')!.trigger('click');
    expect(t.closed).toEqual([true]);
    expect(deck.closeDialogOpen.value).toBe(false);
  });

  it('Stop and close calls close(false)', async () => {
    const { t, deck, w } = setup([sess('a', 'working', 'A')]);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    await w.findAll('button').find((b) => b.text() === 'Stop and close')!.trigger('click');
    expect(t.closed).toEqual([false]);
  });

  it('Cancel and Escape dismiss without closing', async () => {
    const { t, deck, w } = setup([sess('a', 'working', 'A')]);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    await w.findAll('button').find((b) => b.text() === 'Cancel')!.trigger('click');
    expect(deck.closeDialogOpen.value).toBe(false);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    await w.get('[role=dialog]').trigger('keydown', { key: 'Escape' });
    expect(deck.closeDialogOpen.value).toBe(false);
    expect(t.closed).toEqual([]);
  });

  it('traps Tab from the last button back to the first', async () => {
    const { deck, w } = setup([sess('a', 'working', 'A')]);
    deck.closeDialogOpen.value = true;
    await w.vm.$nextTick();
    await w.vm.$nextTick();
    await w.vm.$nextTick();
    const buttons = w.findAll('button');
    (buttons[buttons.length - 1]!.element as HTMLElement).focus();
    await w.get('[role=dialog]').trigger('keydown', { key: 'Tab' });
    expect(document.activeElement).toBe(buttons[0]!.element);
  });
});
