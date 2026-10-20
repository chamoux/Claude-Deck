<script setup lang="ts">
import { computed } from 'vue';
import { Moon } from 'lucide-vue-next';
import { useDeck } from '../useDeck';

/** Shown in a pane whose active session is asleep. Waking is explicit: only then does a terminal attach. */
const props = defineProps<{ deckId: string }>();
const deck = useDeck();

const session = computed(() => deck.session(props.deckId));
const name = computed(() => session.value?.name || 'New session');
</script>

<template>
  <div data-testid="asleep-panel" class="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
    <Moon :size="40" :stroke-width="1.5" class="text-deck-faint" aria-hidden="true" />
    <h2 class="m-0 max-w-full truncate text-[15px] font-semibold text-deck-text">{{ name }}</h2>
    <p class="m-0 text-deck-muted">This session is asleep</p>
    <p v-if="session?.error" data-testid="asleep-error" class="m-0 max-w-sm text-xs text-deck-danger">{{ session.error }}</p>
    <button
      type="button"
      data-testid="wake"
      class="h-8 cursor-pointer rounded-md bg-deck-accent px-3 text-[13px] font-medium text-white transition-colors duration-150 hover:bg-deck-accent-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:ring-offset-0"
      @click="deck.wakeSession(deckId)"
    >
      Wake
    </button>
  </div>
</template>
