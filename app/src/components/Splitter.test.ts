import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Splitter from './Splitter.vue';

// jsdom has no PointerEvent and test-utils cannot set clientX on a MouseEvent, so dispatch by hand.
function ptr(w: { element: Element }, type: string, init: { clientX: number; button?: number }): Promise<void> {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, ...init });
  Object.defineProperty(e, 'pointerId', { value: 1 });
  w.element.dispatchEvent(e);
  return Promise.resolve().then(() => undefined);
}


describe('Splitter', () => {
  it('has separator semantics', () => {
    const w = mount(Splitter);
    expect(w.attributes('role')).toBe('separator');
    expect(w.attributes('aria-orientation')).toBe('vertical');
    expect(w.attributes('tabindex')).toBe('0');
  });

  it('emits cumulative drag deltas then end', async () => {
    const w = mount(Splitter);
    await ptr(w, 'pointerdown', { clientX: 100 });
    await ptr(w, 'pointermove', { clientX: 90 });
    await ptr(w, 'pointermove', { clientX: 70 });
    await ptr(w, 'pointerup', { clientX: 70 });
    expect(w.emitted('drag')).toEqual([[-10], [-30]]);
    expect(w.emitted('end')).toHaveLength(1);
  });

  it('ignores moves without a pointer down', async () => {
    const w = mount(Splitter);
    await ptr(w, 'pointermove', { clientX: 50 });
    expect(w.emitted('drag')).toBeUndefined();
  });

  it('adjusts by 16 px with Left and Right', async () => {
    const w = mount(Splitter);
    await w.trigger('keydown', { key: 'ArrowLeft' });
    await w.trigger('keydown', { key: 'ArrowRight' });
    await w.trigger('keydown', { key: 'a' });
    expect(w.emitted('drag')).toEqual([[-16], [16]]);
    expect(w.emitted('end')).toHaveLength(2);
  });

  it('emits reset on double click', async () => {
    const w = mount(Splitter);
    await w.trigger('dblclick');
    expect(w.emitted('reset')).toHaveLength(1);
  });
});
