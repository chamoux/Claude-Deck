<script setup lang="ts">
import { Clock, X } from 'lucide-vue-next';
import { nextTick, ref, watch } from 'vue';
import { useDeck } from '../useDeck';

const props = defineProps<{ projectId: string; open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const deck = useDeck();
const dialog = ref<HTMLElement | null>(null);
let previouslyFocused: HTMLElement | null = null;

const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 31_536_000_000],
  ['month', 2_592_000_000],
  ['day', 86_400_000],
  ['hour', 3_600_000],
  ['minute', 60_000],
];

function relative(ms: number): string {
  const diff = ms - Date.now();
  for (const [unit, size] of UNITS) {
    if (Math.abs(diff) >= size) return rtf.format(Math.round(diff / size), unit);
  }
  return 'just now';
}

function focusables(): HTMLElement[] {
  return Array.from(dialog.value?.querySelectorAll<HTMLElement>('button:not([disabled])') ?? []);
}

watch(
  () => [props.open, props.projectId] as const,
  async ([open], old) => {
    const wasOpen = old?.[0] ?? false;
    if (!open) {
      if (wasOpen) previouslyFocused?.focus();
      return;
    }
    if (!wasOpen) previouslyFocused = document.activeElement as HTMLElement | null;
    deck.listPast(props.projectId);
    await nextTick();
    focusables()[0]?.focus();
  },
  { immediate: true },
);

function onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    e.preventDefault();
    emit('close');
    return;
  }
  if (e.key !== 'Tab') return;
  const items = focusables();
  const first = items[0];
  const last = items[items.length - 1];
  if (!first || !last) return;
  const active = document.activeElement;
  if (e.shiftKey && active === first) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && active === last) {
    e.preventDefault();
    first.focus();
  } else if (!dialog.value?.contains(active)) {
    e.preventDefault();
    first.focus();
  }
}

function pick(id: string): void {
  deck.openPast(props.projectId, id);
  emit('close');
}
</script>

<template>
  <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center bg-black/60" @click.self="emit('close')">
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="past-sessions-title"
      class="flex max-h-[70vh] w-120 max-w-[90vw] flex-col rounded-xl border border-deck-border-strong bg-deck-elevated text-[13px] text-deck-text shadow-2xl"
      @keydown="onKeydown"
    >
      <div class="flex items-start gap-3 p-5 pb-3">
        <div class="min-w-0 flex-1">
          <h2 id="past-sessions-title" class="text-[15px] font-semibold">Resume a past conversation</h2>
          <p class="mt-1 text-deck-muted">Pick an earlier Claude conversation from this folder to continue it.</p>
        </div>
        <button
          type="button"
          aria-label="Close"
          title="Close"
          class="-mt-1 -mr-1 flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
          @click="emit('close')"
        >
          <X :size="16" :stroke-width="1.75" aria-hidden="true" />
        </button>
      </div>
      <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        <p v-if="(deck.pastSessions[projectId] ?? []).length === 0" class="px-3 py-8 text-center text-deck-muted">
          No other conversations for this folder
        </p>
        <ul v-else>
          <li v-for="s in deck.pastSessions[projectId]" :key="s.id">
            <button
              type="button"
              class="flex h-11 w-full cursor-pointer flex-col justify-center rounded-md px-3 text-left transition-colors duration-150 hover:bg-deck-hover focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
              @click="pick(s.id)"
            >
              <span v-if="s.last_prompt" class="block w-full truncate leading-5">{{ s.last_prompt }}</span>
              <span v-else class="block w-full truncate leading-5 text-deck-faint italic">No prompt</span>
              <span class="flex items-center gap-1 text-xs leading-4 text-deck-faint">
                <Clock :size="12" :stroke-width="1.75" aria-hidden="true" />
                {{ relative(s.modified_ms) }}
              </span>
            </button>
          </li>
        </ul>
      </div>
    </div>
  </div>
</template>
