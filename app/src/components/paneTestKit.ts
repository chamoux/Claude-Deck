import { mount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import type { Component } from 'vue';
import type { LayoutNode, Pane, TabRef } from '../layout';
import type { Group, Project, SessionState, SessionView, StateView } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import type { ViewRef } from '../useDeck';

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };

export const pane = (id: string, tabs: TabRef[], active = 0): Pane => ({ type: 'pane', id, tabs, active });
export const sTab = (deck_id: string): TabRef => ({ kind: 'session', deck_id });
export const split = (dir: 'row' | 'column', a: LayoutNode, b: LayoutNode, ratio = 0.5): LayoutNode => ({
  type: 'split',
  dir,
  ratio,
  a,
  b,
});

export function sess(
  deck_id: string,
  project_id: string,
  state: SessionState = 'idle',
  name: string | null = null,
  error: string | null = null,
): SessionView {
  return { deck_id, project_id, name, state, background_work: false, error };
}

export interface KitOptions {
  sessions: SessionView[];
  projectLayouts?: Record<string, LayoutNode>;
  groups?: Group[];
  select?: ViewRef;
}

/** A deck loaded with projects p1 ("alpha") and p2 ("beta"), the given sessions/layouts, and a view selected. */
export function buildDeck(o: KitOptions) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  const projects: Project[] = ['p1', 'p2'].map((id) => ({
    id,
    path: `C:/code/${id}`,
    name: id === 'p1' ? 'alpha' : 'beta',
    shell_panel: { open: false, width: 480 },
    layout: o.projectLayouts?.[id] ?? null,
  }));
  t.emit({
    type: 'state',
    state: { projects, sessions: o.sessions, shells: [], settings, missing_folders: [], groups: o.groups ?? [] },
  });
  const sel = o.select ?? { kind: 'project', id: 'p1' };
  if (sel.kind === 'project') deck.selectProject(sel.id);
  else deck.selectGroup(sel.id);
  t.sent.length = 0;
  return { t, deck };
}

export function mountWithDeck(deck: ReturnType<typeof createDeck>, component: Component, props: Record<string, unknown> | (() => Record<string, unknown>)) {
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(component, typeof props === 'function' ? props() : props);
    },
  });
  return mount(Host, { attachTo: document.body });
}

/** Dispatches a drag-like event (jsdom has no DragEvent) with coordinates and a dataTransfer double. */
export function dragEvent(
  el: Element,
  type: string,
  opts: { x?: number; y?: number; data?: Record<string, string> } = {},
): { event: MouseEvent; store: Record<string, string>; dt: DataTransfer } {
  const store: Record<string, string> = { ...(opts.data ?? {}) };
  const dt: { types: string[]; effectAllowed: string; dropEffect: string; setData(k: string, v: string): void; getData(k: string): string } = {
    types: Object.keys(store),
    effectAllowed: 'uninitialized',
    dropEffect: 'none',
    setData(k: string, v: string) {
      store[k] = v;
      this.types = Object.keys(store);
    },
    getData: (k: string) => store[k] ?? '',
  };
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: opts.x ?? 0, clientY: opts.y ?? 0 });
  Object.defineProperty(event, 'dataTransfer', { value: dt });
  el.dispatchEvent(event);
  return { event, store, dt: dt as unknown as DataTransfer };
}
