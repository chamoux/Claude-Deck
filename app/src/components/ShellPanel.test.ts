import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h } from 'vue';
import type { Project, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import ShellPanel from './ShellPanel.vue';

// jsdom has no PointerEvent and test-utils cannot set clientX on a MouseEvent, so dispatch by hand.
function ptr(w: { element: Element }, type: string, init: { clientX: number; button?: number }): Promise<void> {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, ...init });
  Object.defineProperty(e, 'pointerId', { value: 1 });
  w.element.dispatchEvent(e);
  return Promise.resolve().then(() => undefined);
}


vi.mock('./TerminalView.vue', () => ({
  default: {
    name: 'TerminalView',
    props: ['target'],
    template: '<div data-testid="term">{{ target.project_id }}</div>',
  },
}));

function project(id: string, open: boolean, width: number): Project {
  return { id, path: `C:/code/${id}`, name: `Name ${id}`, shell_panel: { open, width } };
}

function setup(props: { projectId: string; containerWidth?: number } = { projectId: 'p1' }, running = false) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  const state: StateView = {
    projects: [project('p1', true, 500), project('p2', true, 300), project('p3', false, 480)],
    sessions: [],
    shells: running ? [{ project_id: 'p3', running: true }] : [],
    settings: { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'git_bash' },
    missing_folders: [],
  };
  t.emit({ type: 'state', state });
  t.sent.length = 0;
  const Host = defineComponent({
    props: ['projectId', 'containerWidth'],
    setup(p) {
      provideDeck(deck);
      return () => h(ShellPanel, { projectId: p.projectId, containerWidth: p.containerWidth });
    },
  });
  const w = mount(Host, { props });
  return { t, deck, w };
}

const style = (w: ReturnType<typeof setup>['w']) => (w.find('[data-testid="shell-open"]').element as HTMLElement).style.width;

describe('ShellPanel', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('open state renders header and a shell TerminalView', () => {
    const { w } = setup();
    const text = w.text();
    expect(text).toContain('Terminal');
    expect(text).toContain('Name p1');
    expect(text).toContain('Git Bash');
    expect(w.findComponent({ name: 'TerminalView' }).props('target')).toEqual({ kind: 'shell', project_id: 'p1' });
    expect(style(w)).toBe('500px');
  });

  it('minimize sends SetShellPanel open:false and the rail renders', async () => {
    const { t, w, deck } = setup();
    await w.find('[aria-label="Minimize terminal"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p1', panel: { open: false, width: 500 } }]);
    t.emit({
      type: 'state',
      state: { ...deck.state.value!, projects: [project('p1', false, 500), project('p2', true, 300), project('p3', false, 480)] },
    });
    await w.vm.$nextTick();
    expect(w.find('[data-testid="shell-rail"]').exists()).toBe(true);
    expect(w.find('[data-testid="term"]').exists()).toBe(false);
  });

  it('clicking the rail sends open:true, and shows a dot when running', async () => {
    const { t, w } = setup({ projectId: 'p3' }, true);
    expect(w.find('[data-testid="shell-running"]').exists()).toBe(true);
    expect(w.find('[data-testid="term"]').exists()).toBe(false);
    await w.find('[data-testid="shell-rail"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p3', panel: { open: true, width: 480 } }]);
  });

  it('no dot when the shell is not running', () => {
    const { w } = setup({ projectId: 'p3' });
    expect(w.find('[data-testid="shell-running"]').exists()).toBe(false);
  });

  it('close asks to confirm, then sends CloseShell', async () => {
    const { t, w, deck } = setup();
    await w.find('[aria-label="Close terminal"]').trigger('click');
    expect(deck.confirm.value?.message).toBe('Close this terminal? Anything running in it stops.');
    expect(t.sent).toEqual([]);
    deck.confirmYes();
    expect(t.sent).toEqual([{ type: 'close_shell', project_id: 'p1' }]);
  });

  it('dragging clamps the width and sends one SetShellPanel after the debounce', async () => {
    const { t, w } = setup({ projectId: 'p1', containerWidth: 1000 }); // max = 580
    const sep = w.find('[role="separator"]');
    await ptr(sep, 'pointerdown', { clientX: 500 });
    await ptr(sep, 'pointermove', { clientX: 450 });
    expect(style(w)).toBe('550px');
    await ptr(sep, 'pointermove', { clientX: 300 });
    expect(style(w)).toBe('580px'); // Claude area keeps 420
    await ptr(sep, 'pointermove', { clientX: 900 });
    expect(style(w)).toBe('240px'); // min
    await ptr(sep, 'pointermove', { clientX: 480 });
    await ptr(sep, 'pointerup', { clientX: 480 });
    expect(t.sent).toEqual([]);
    vi.advanceTimersByTime(299);
    expect(t.sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p1', panel: { open: true, width: 520 } }]);
  });

  it('caps at 1600 without a container width', async () => {
    const { w } = setup();
    const sep = w.find('[role="separator"]');
    await ptr(sep, 'pointerdown', { clientX: 5000 });
    await ptr(sep, 'pointermove', { clientX: 0 });
    expect(style(w)).toBe('1600px');
  });

  it('double click resets to 480', async () => {
    const { t, w } = setup();
    await w.find('[role="separator"]').trigger('dblclick');
    expect(style(w)).toBe('480px');
    vi.advanceTimersByTime(300);
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p1', panel: { open: true, width: 480 } }]);
  });

  it('changing projectId switches target and stored width', async () => {
    const { w } = setup();
    await w.setProps({ projectId: 'p2' });
    expect(w.findComponent({ name: 'TerminalView' }).props('target')).toEqual({ kind: 'shell', project_id: 'p2' });
    expect(style(w)).toBe('300px');
    expect(w.text()).toContain('Name p2');
  });

  it('flushes a pending width save when switching projects', async () => {
    const { t, w } = setup();
    const sep = w.find('[role="separator"]');
    await ptr(sep, 'pointerdown', { clientX: 500 });
    await ptr(sep, 'pointermove', { clientX: 460 });
    await ptr(sep, 'pointerup', { clientX: 460 });
    await w.setProps({ projectId: 'p2' });
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p1', panel: { open: true, width: 540 } }]);
    vi.advanceTimersByTime(500);
    expect(t.sent).toHaveLength(1);
  });
});
