import { mount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import { describe, expect, it } from 'vitest';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import StatusFooter from './StatusFooter.vue';

function setup() {
  const t = fakeTransport();
  const deck = createDeck(t);
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(StatusFooter);
    },
  });
  return { t, deck, w: mount(Host) };
}

const GB = 1024 ** 3;
const MB = 1024 ** 2;

describe('StatusFooter', () => {
  it('shows a dash without memory data', () => {
    expect(setup().w.get('[data-testid=memory]').text()).toBe('–');
  });

  it('formats GB with one decimal at or above 1 GiB', async () => {
    const { deck, w } = setup();
    deck.memory.value = { awake: 3, bytes: Math.round(1.8 * GB) };
    await w.vm.$nextTick();
    expect(w.get('[data-testid=memory]').text()).toBe('3 awake · 1.8 GB');
    deck.memory.value = { awake: 1, bytes: GB };
    await w.vm.$nextTick();
    expect(w.get('[data-testid=memory]').text()).toBe('1 awake · 1.0 GB');
  });

  it('formats MB with no decimals below 1 GiB', async () => {
    const { deck, w } = setup();
    deck.memory.value = { awake: 1, bytes: 640 * MB };
    await w.vm.$nextTick();
    expect(w.get('[data-testid=memory]').text()).toBe('1 awake · 640 MB');
  });

  it('shows nothing extra when connected', () => {
    expect(setup().w.find('[role=status]').exists()).toBe(false);
  });

  it('shows Reconnecting in amber', async () => {
    const { deck, w } = setup();
    deck.connection.value = 'reconnecting';
    await w.vm.$nextTick();
    const s = w.get('[role=status]');
    expect(s.text()).toBe('Reconnecting…');
    expect(s.classes()).toContain('text-dot-needs');
  });

  it('shows Helper not running in red when failed', async () => {
    const { deck, w } = setup();
    deck.connection.value = 'failed';
    await w.vm.$nextTick();
    const s = w.get('[role=status]');
    expect(s.text()).toBe('Helper not running');
    expect(s.classes()).toContain('text-deck-danger');
  });

  it('emits open-settings from the footer gear', async () => {
    const { w } = setup();
    await w.get('[data-testid=open-settings]').trigger('click');
    expect(w.findComponent(StatusFooter).emitted('open-settings')).toHaveLength(1);
  });

  it('shows the last error and dismisses it', async () => {
    const { deck, w } = setup();
    expect(w.find('[data-testid=error]').exists()).toBe(false);
    deck.lastError.value = 'git exploded';
    await w.vm.$nextTick();
    expect(w.get('[data-testid=error]').text()).toBe('git exploded');
    await w.get('button[aria-label="Dismiss error"]').trigger('click');
    expect(deck.lastError.value).toBeNull();
    expect(w.find('[data-testid=error]').exists()).toBe(false);
  });
});
