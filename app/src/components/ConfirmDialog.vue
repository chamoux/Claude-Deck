<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { AlertTriangle } from 'lucide-vue-next';
import { useDeck } from '../useDeck';

const deck = useDeck();
const panel = ref<HTMLElement | null>(null);
const yesBtn = ref<HTMLButtonElement | null>(null);

// Danger rule: the confirm button is red when the message names a destructive
// action (remove, close, stop, delete, discard); otherwise it uses the primary style.
const DANGER = /\b(remove|close|stop|delete|discard)\b/i;
const danger = computed(() => DANGER.test(deck.confirm.value?.message ?? ''));

watch(
  () => deck.confirm.value,
  async (c) => {
    if (!c) return;
    await nextTick();
    yesBtn.value?.focus();
  },
  { immediate: true, flush: 'post' },
);

function trap(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    e.preventDefault();
    deck.confirmNo();
    return;
  }
  if (e.key !== 'Tab' || !panel.value) return;
  const f = Array.from(panel.value.querySelectorAll<HTMLElement>('button:not([disabled])'));
  const first = f[0];
  const last = f[f.length - 1];
  if (!first || !last) return;
  if (e.shiftKey && document.activeElement === first) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && document.activeElement === last) {
    e.preventDefault();
    first.focus();
  }
}
</script>

<template>
  <Transition
    enter-active-class="transition-opacity duration-150 motion-reduce:transition-none"
    enter-from-class="opacity-0 *:scale-95 motion-reduce:*:scale-100"
  >
    <div v-if="deck.confirm.value" class="fixed inset-0 z-50 flex items-center justify-center bg-black/60" @keydown="trap">
      <div
        ref="panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        class="w-110 max-w-[90vw] rounded-xl border border-deck-border-strong bg-deck-elevated p-5 text-deck-text shadow-2xl transition-transform duration-150 motion-reduce:transition-none"
      >
        <div class="flex items-start gap-3">
          <AlertTriangle :size="18" :stroke-width="1.75" class="mt-px shrink-0 text-dot-needs" aria-hidden="true" />
          <h2 id="confirm-dialog-title" class="text-[15px] leading-snug font-semibold">{{ deck.confirm.value.message }}</h2>
        </div>
        <div class="mt-5 flex justify-end gap-2">
          <button
            type="button"
            class="h-8 cursor-pointer rounded-md px-3 whitespace-nowrap text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
            @click="deck.confirmNo()"
          >
            Cancel
          </button>
          <button
            ref="yesBtn"
            type="button"
            :data-danger="danger"
            class="h-8 cursor-pointer rounded-md px-3 font-medium whitespace-nowrap transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-white/80 focus-visible:outline-none"
            :class="
              danger
                ? 'bg-deck-danger text-deck-bg hover:brightness-110'
                : 'bg-deck-accent text-white hover:bg-deck-accent-hover'
            "
            @click="deck.confirmYes()"
          >
            Yes, continue
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
