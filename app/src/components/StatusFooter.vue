<script setup lang="ts">
import { computed } from 'vue';
import { Cpu, Settings, X } from 'lucide-vue-next';
import { useDeck } from '../useDeck';
import UsageMeters from './UsageMeters.vue';

const emit = defineEmits<{ 'open-settings': [] }>();
const deck = useDeck();

function formatBytes(bytes: number): string {
  const gib = 1024 ** 3;
  if (bytes >= gib) return `${(bytes / gib).toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

const memoryText = computed(() => {
  const m = deck.memory.value;
  return m ? `${m.awake} awake · ${formatBytes(m.bytes)}` : '–';
});

const helper = computed(() => {
  switch (deck.connection.value) {
    case 'reconnecting':
      return { dot: 'bg-dot-needs', label: 'Helper reconnecting' };
    case 'failed':
      return { dot: 'bg-deck-danger', label: 'Helper not running' };
    default:
      return { dot: 'bg-dot-working', label: 'Helper connected' };
  }
});
</script>

<template>
  <footer class="flex h-[26px] shrink-0 items-center gap-4 border-t border-deck-border bg-deck-panel pr-1 pl-3 text-xs text-deck-muted">
    <span class="flex shrink-0 items-center gap-1.5" data-testid="helper">
      <span :class="['size-2 rounded-full', helper.dot]" :title="helper.label" :aria-label="helper.label" role="img" />
      <span>Helper</span>
    </span>
    <span v-if="deck.connection.value === 'reconnecting'" class="shrink-0 text-dot-needs" role="status">Reconnecting…</span>
    <span v-else-if="deck.connection.value === 'failed'" class="shrink-0 text-deck-danger" role="status">Helper not running</span>
    <span class="flex shrink-0 items-center gap-1.5">
      <Cpu :size="14" :stroke-width="1.75" class="text-deck-faint" aria-hidden="true" />
      <span data-testid="memory" class="font-mono">{{ memoryText }}</span>
    </span>
    <span class="flex min-w-0 flex-1 items-center justify-center">
      <span v-if="deck.lastError.value" class="flex max-w-full min-w-0 items-center gap-1 text-deck-danger">
        <span class="truncate" :title="deck.lastError.value" data-testid="error">{{ deck.lastError.value }}</span>
        <button
          type="button"
          aria-label="Dismiss error"
          title="Dismiss error"
          class="flex size-5 shrink-0 cursor-pointer items-center justify-center rounded transition-colors duration-150 hover:bg-deck-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
          @click="deck.lastError.value = null"
        >
          <X :size="14" :stroke-width="1.75" aria-hidden="true" />
        </button>
      </span>
    </span>
    <UsageMeters />
    <button
      type="button"
      data-testid="open-settings"
      aria-label="Settings"
      title="Settings"
      class="flex size-[22px] shrink-0 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
      @click="emit('open-settings')"
    >
      <Settings :size="16" :stroke-width="1.75" aria-hidden="true" />
    </button>
  </footer>
</template>
