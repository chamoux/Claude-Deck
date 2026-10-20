import { mount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import ConfirmDialog from './ConfirmDialog.vue';

function setup() {
  const deck = createDeck(fakeTransport());
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(ConfirmDialog);
    },
  });
  return { deck, w: mount(Host, { attachTo: document.body }) };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('ConfirmDialog', () => {
  it('renders nothing without a pending confirm', () => {
    expect(setup().w.find('[role=dialog]').exists()).toBe(false);
  });

  it('shows the message as an accessible modal and focuses Yes', async () => {
    const { deck, w } = setup();
    deck.confirm.value = { message: 'Claude may be working. Stop it anyway?', yes: vi.fn() };
    await w.vm.$nextTick();
    await w.vm.$nextTick();
    const dlg = w.get('[role=dialog]');
    expect(dlg.attributes('aria-modal')).toBe('true');
    expect(w.get(`#${dlg.attributes('aria-labelledby')}`).text()).toBe('Claude may be working. Stop it anyway?');
    expect(document.activeElement?.textContent?.trim()).toBe('Yes, continue');
  });

  it('Yes runs the callback and clears the confirm', async () => {
    const { deck, w } = setup();
    const yes = vi.fn();
    deck.confirm.value = { message: 'Sure?', yes };
    await w.vm.$nextTick();
    await w.findAll('button').find((b) => b.text() === 'Yes, continue')!.trigger('click');
    expect(yes).toHaveBeenCalledOnce();
    expect(deck.confirm.value).toBeNull();
  });

  it('Cancel and Escape dismiss without running the callback', async () => {
    const { deck, w } = setup();
    const yes = vi.fn();
    deck.confirm.value = { message: 'Sure?', yes };
    await w.vm.$nextTick();
    await w.findAll('button').find((b) => b.text() === 'Cancel')!.trigger('click');
    expect(deck.confirm.value).toBeNull();
    deck.confirm.value = { message: 'Again?', yes };
    await w.vm.$nextTick();
    await w.get('[role=dialog]').trigger('keydown', { key: 'Escape' });
    expect(deck.confirm.value).toBeNull();
    expect(yes).not.toHaveBeenCalled();
  });
});
