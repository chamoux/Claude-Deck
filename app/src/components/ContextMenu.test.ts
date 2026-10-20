import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import ContextMenu from './ContextMenu.vue';

const items = [
  { key: 'a', label: 'Alpha' },
  { key: 'b', label: 'Beta', disabled: true },
  { key: 'c', label: 'Gamma' },
];

function make() {
  return mount(ContextMenu, { props: { x: 10, y: 20, items }, attachTo: document.body });
}

describe('ContextMenu', () => {
  it('renders items at the position', () => {
    const w = make();
    expect(w.findAll('[role="menuitem"]').map((i) => i.text())).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(w.get('[role="menu"]').attributes('style')).toContain('left: 10px');
    w.unmount();
  });

  it('emits select and close on click', async () => {
    const w = make();
    await w.findAll('[role="menuitem"]')[0].trigger('click');
    expect(w.emitted('select')![0]).toEqual(['a']);
    expect(w.emitted('close')).toBeTruthy();
    w.unmount();
  });

  it('does not select disabled items', async () => {
    const w = make();
    await w.findAll('[role="menuitem"]')[1].trigger('click');
    expect(w.emitted('select')).toBeUndefined();
    w.unmount();
  });

  it('closes on Escape', async () => {
    const w = make();
    await w.get('[role="menu"]').trigger('keydown', { key: 'Escape' });
    expect(w.emitted('close')).toBeTruthy();
    w.unmount();
  });

  it('closes on outside click', () => {
    const w = make();
    document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(w.emitted('close')).toBeTruthy();
    w.unmount();
  });

  it('arrow keys move focus and skip disabled items', async () => {
    const w = make();
    const menu = w.get('[role="menu"]');
    const els = w.findAll('[role="menuitem"]').map((e) => e.element as HTMLElement);
    expect(document.activeElement).toBe(els[0]);
    await menu.trigger('keydown', { key: 'ArrowDown' });
    expect(document.activeElement).toBe(els[2]);
    await menu.trigger('keydown', { key: 'ArrowUp' });
    expect(document.activeElement).toBe(els[0]);
    w.unmount();
  });

  describe('submenus', () => {
    const subItems = [
      { key: 'a', label: 'Alpha' },
      { key: 'g', label: 'Add to group', children: [{ key: 'g1', label: 'One' }, { key: 'g2', label: 'Two' }] },
      { key: 'z', label: 'Zed' },
    ];
    const mk = () => mount(ContextMenu, { props: { x: 10, y: 20, items: subItems }, attachTo: document.body });
    const trigger = (w: ReturnType<typeof mk>) => w.findAll('[role="menuitem"]')[1];
    const subItemEls = (w: ReturnType<typeof mk>) => w.findAll('[data-submenu] [role="menuitem"]');

    it('a parent item opens its submenu and does not select', async () => {
      const w = mk();
      expect(trigger(w).attributes('aria-haspopup')).toBe('menu');
      expect(subItemEls(w)).toHaveLength(0);
      await trigger(w).trigger('click');
      expect(w.emitted('select')).toBeUndefined();
      expect(w.emitted('close')).toBeUndefined();
      expect(subItemEls(w).map((e) => e.text())).toEqual(['One', 'Two']);
      w.unmount();
    });

    it('selecting a child emits its key and closes', async () => {
      const w = mk();
      await trigger(w).trigger('click');
      await subItemEls(w)[1].trigger('click');
      expect(w.emitted('select')![0]).toEqual(['g2']);
      expect(w.emitted('close')).toBeTruthy();
      w.unmount();
    });

    it('ArrowRight opens the submenu and focuses its first item', async () => {
      const w = mk();
      (trigger(w).element as HTMLElement).focus();
      await trigger(w).trigger('keydown', { key: 'ArrowRight' });
      await nextTick();
      expect(trigger(w).attributes('aria-expanded')).toBe('true');
      expect(document.activeElement).toBe(subItemEls(w)[0].element);
      w.unmount();
    });

    it('arrow keys move inside the submenu only', async () => {
      const w = mk();
      (trigger(w).element as HTMLElement).focus();
      await trigger(w).trigger('keydown', { key: 'ArrowRight' });
      await nextTick();
      await subItemEls(w)[0].trigger('keydown', { key: 'ArrowDown' });
      expect(document.activeElement).toBe(subItemEls(w)[1].element);
      await subItemEls(w)[1].trigger('keydown', { key: 'ArrowDown' });
      expect(document.activeElement).toBe(subItemEls(w)[0].element);
      w.unmount();
    });

    it('ArrowLeft closes the submenu and refocuses the parent', async () => {
      const w = mk();
      (trigger(w).element as HTMLElement).focus();
      await trigger(w).trigger('keydown', { key: 'ArrowRight' });
      await nextTick();
      await subItemEls(w)[0].trigger('keydown', { key: 'ArrowLeft' });
      expect(subItemEls(w)).toHaveLength(0);
      expect(document.activeElement).toBe(trigger(w).element);
      expect(w.emitted('close')).toBeUndefined();
      w.unmount();
    });

    it('Escape in the submenu closes only the submenu; a second Escape closes the menu', async () => {
      const w = mk();
      (trigger(w).element as HTMLElement).focus();
      await trigger(w).trigger('keydown', { key: 'ArrowRight' });
      await nextTick();
      await subItemEls(w)[0].trigger('keydown', { key: 'Escape' });
      expect(subItemEls(w)).toHaveLength(0);
      expect(w.emitted('close')).toBeUndefined();
      await trigger(w).trigger('keydown', { key: 'Escape' });
      expect(w.emitted('close')).toBeTruthy();
      w.unmount();
    });

    it('hovering a parent opens the submenu after 150 ms', async () => {
      vi.useFakeTimers();
      const w = mk();
      await trigger(w).trigger('mouseenter');
      vi.advanceTimersByTime(100);
      await nextTick();
      expect(subItemEls(w)).toHaveLength(0);
      vi.advanceTimersByTime(60);
      await nextTick();
      expect(subItemEls(w)).toHaveLength(2);
      w.unmount();
      vi.useRealTimers();
    });

    it('hovering another item cancels or closes the submenu', async () => {
      vi.useFakeTimers();
      const w = mk();
      await trigger(w).trigger('mouseenter');
      await w.findAll('[role="menuitem"]')[0].trigger('mouseenter');
      vi.advanceTimersByTime(300);
      await nextTick();
      expect(subItemEls(w)).toHaveLength(0);
      await trigger(w).trigger('mouseenter');
      vi.advanceTimersByTime(200);
      await nextTick();
      expect(subItemEls(w)).toHaveLength(2);
      await w.findAll('[role="menuitem"]')[0].trigger('mouseenter');
      await nextTick();
      expect(subItemEls(w)).toHaveLength(0);
      w.unmount();
      vi.useRealTimers();
    });

    it('opens to the right of the parent and flips left near the window edge', async () => {
      const rect = (l: number, t: number, w: number, h: number) =>
        ({ left: l, top: t, right: l + w, bottom: t + h, width: w, height: h, x: l, y: t, toJSON: () => ({}) }) as DOMRect;
      const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
        if (this.hasAttribute('data-submenu')) return rect(0, 0, 160, 80);
        return rect(800, 50, 150, 28);
      });
      const old = window.innerWidth;
      Object.defineProperty(window, 'innerWidth', { value: 1000, configurable: true });
      const w = mk();
      await trigger(w).trigger('click');
      await nextTick();
      await nextTick();
      expect(w.get('[data-submenu]').attributes('style')).toContain('left: 640px');
      Object.defineProperty(window, 'innerWidth', { value: old, configurable: true });
      spy.mockRestore();
      w.unmount();
    });
  });
});
