import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { defineComponent, h } from 'vue';
import type { PastSession, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import PastSessionsDialog from './PastSessionsDialog.vue';

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };
const DAY = 86_400_000;

function setup(open = true) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({ type: 'state', state: { projects: [], sessions: [], shells: [], settings, missing_folders: [] } });
  t.sent.length = 0;
  const Host = defineComponent({
    props: { open: { type: Boolean, default: open } },
    emits: ['close'],
    setup(props, { emit }) {
      provideDeck(deck);
      return () => h(PastSessionsDialog, { projectId: 'p1', open: props.open, onClose: () => emit('close') });
    },
  });
  const w = mount(Host, { attachTo: document.body });
  return { t, w };
}

const past = (id: string, last_prompt: string | null, daysAgo: number): PastSession => ({
  id,
  last_prompt,
  modified_ms: Date.now() - daysAgo * DAY,
});

describe('PastSessionsDialog', () => {
  it('sends ListPastSessions on open', () => {
    const { t, w } = setup();
    expect(t.sent).toEqual([{ type: 'list_past_sessions', project_id: 'p1' }]);
    w.unmount();
  });

  it('renders nothing while closed and lists when opened later', async () => {
    const { t, w } = setup(false);
    expect(t.sent).toEqual([]);
    expect(w.find('[role="dialog"]').exists()).toBe(false);
    await w.setProps({ open: true });
    expect(t.sent).toEqual([{ type: 'list_past_sessions', project_id: 'p1' }]);
    expect(w.find('[role="dialog"]').attributes('aria-modal')).toBe('true');
    w.unmount();
  });

  it('renders prompts with relative dates, and No prompt in italic', async () => {
    const { t, w } = setup();
    t.emit({ type: 'past_sessions', project_id: 'p1', sessions: [past('a', 'Fix the cart bug', 3), past('b', null, 1)] });
    await w.vm.$nextTick();
    const rows = w.findAll('li button');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain('Fix the cart bug');
    expect(rows[0]!.text()).toContain('3 days ago');
    expect(rows[1]!.text()).toContain('No prompt');
    expect(rows[1]!.text()).toContain('yesterday');
    expect(rows[1]!.find('.italic').exists()).toBe(true);
    w.unmount();
  });

  it('click sends OpenPastSession and emits close', async () => {
    const { t, w } = setup();
    t.emit({ type: 'past_sessions', project_id: 'p1', sessions: [past('conv-1', 'hello', 2)] });
    await w.vm.$nextTick();
    t.sent.length = 0;
    await w.find('li button').trigger('click');
    expect(t.sent).toEqual([{ type: 'open_past_session', project_id: 'p1', conversation_id: 'conv-1' }]);
    expect(w.emitted('close')).toHaveLength(1);
    w.unmount();
  });

  it('shows the empty state', async () => {
    const { t, w } = setup();
    t.emit({ type: 'past_sessions', project_id: 'p1', sessions: [] });
    await w.vm.$nextTick();
    expect(w.text()).toContain('No other conversations for this folder');
    w.unmount();
  });

  it('Escape closes', async () => {
    const { w } = setup();
    await w.find('[role="dialog"]').trigger('keydown', { key: 'Escape' });
    expect(w.emitted('close')).toHaveLength(1);
    w.unmount();
  });

  it('traps Tab focus inside the dialog', async () => {
    const { t, w } = setup();
    t.emit({ type: 'past_sessions', project_id: 'p1', sessions: [past('a', 'x', 1)] });
    await w.vm.$nextTick();
    const buttons = w.findAll('button');
    (buttons[buttons.length - 1]!.element as HTMLElement).focus();
    await w.find('[role="dialog"]').trigger('keydown', { key: 'Tab' });
    expect(document.activeElement).toBe(buttons[0]!.element);
    w.unmount();
  });
});
