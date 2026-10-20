<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import { AlertTriangle } from 'lucide-vue-next';
import { useDeck } from '../useDeck';

const deck = useDeck();
const panel = ref<HTMLElement | null>(null);
const keepBtn = ref<HTMLButtonElement | null>(null);

const DOT: Record<string, string> = { working: 'bg-dot-working', needs_you: 'bg-dot-needs' };

watch(
  deck.closeDialogOpen,
  async (open) => {
    if (!open) return;
    await nextTick();
    keepBtn.value?.focus();
  },
  { immediate: true, flush: 'post' },
);

function cancel(): void {
  deck.closeDialogOpen.value = false;
}

function trap(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    e.preventDefault();
    cancel();
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
    <div v-if="deck.closeDialogOpen.value" class="fixed inset-0 z-50 flex items-center justify-center bg-black/60" @keydown="trap">
      <div
        ref="panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="close-dialog-title"
        class="w-120 max-w-[90vw] rounded-xl border border-deck-border-strong bg-deck-elevated p-5 text-deck-text shadow-2xl transition-transform duration-150 motion-reduce:transition-none"
      >
        <div class="flex items-center gap-2">
          <AlertTriangle :size="16" :stroke-width="1.75" class="shrink-0 text-dot-needs" aria-hidden="true" />
          <h2 id="close-dialog-title" class="text-[15px] font-semibold">
            {{ deck.busySessions.value.length }}
            {{ deck.busySessions.value.length === 1 ? 'session is' : 'sessions are' }} still working
          </h2>
        </div>
        <p class="mt-1 text-xs text-deck-muted">
          Keep them running and the window closes; they finish in the background and you can reopen later.
        </p>
        <ul class="mt-4 flex max-h-48 flex-col gap-0.5 overflow-y-auto rounded-lg border border-deck-border bg-deck-bg p-1.5">
          <li v-for="s in deck.busySessions.value" :key="s.deck_id" class="flex h-7 items-center gap-2 px-1.5">
            <span class="size-2 shrink-0 rounded-full" :class="DOT[s.state] ?? 'bg-dot-idle'" :data-state="s.state" />
            <span class="min-w-0 flex-1 truncate font-medium" data-name>{{ s.name ?? 'Untitled session' }}</span>
            <span
              v-if="s.state === 'needs_you'"
              class="shrink-0 rounded-full bg-needs-soft px-2 text-[11px] leading-5 font-medium text-dot-needs"
            >
              Needs you
            </span>
          </li>
        </ul>
        <div class="mt-5 flex justify-end gap-2">
          <button
            type="button"
            class="h-8 cursor-pointer rounded-md px-3 whitespace-nowrap text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
            @click="cancel"
          >
            Cancel
          </button>
          <button
            type="button"
            class="h-8 cursor-pointer rounded-md border border-deck-border-strong px-3 whitespace-nowrap transition-colors duration-150 hover:bg-deck-hover focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
            @click="deck.finishClose(false)"
          >
            Stop and close
          </button>
          <button
            ref="keepBtn"
            type="button"
            class="h-8 cursor-pointer rounded-md bg-deck-accent px-3 font-medium whitespace-nowrap text-white transition-colors duration-150 hover:bg-deck-accent-hover focus-visible:ring-2 focus-visible:ring-white/80 focus-visible:outline-none"
            @click="deck.finishClose(true)"
          >
            Keep running in background
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
