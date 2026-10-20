<script setup lang="ts">
import { computed } from 'vue';
import { LayoutGrid } from 'lucide-vue-next';
import type { Pane } from '../layout';
import type { Target } from '../protocol';
import { useDeck } from '../useDeck';
import AsleepPanel from './AsleepPanel.vue';
import ChangesTab from './ChangesTab.vue';
import DropOverlay from './DropOverlay.vue';
import PaneTabs from './PaneTabs.vue';
import TerminalView from './TerminalView.vue';

/**
 * One pane: its tab strip and a body showing the active tab (terminal, asleep panel or changes).
 * Pressing or focusing anywhere inside focuses the pane. The drop overlay lives in the body, above the terminal.
 */
const props = defineProps<{ pane: Pane }>();
const deck = useDeck();

const focused = computed(() => deck.focusedPaneId.value === props.pane.id);
const tab = computed(() => props.pane.tabs[props.pane.active]);
const session = computed(() => (tab.value?.kind === 'session' ? deck.session(tab.value.deck_id) : undefined));
const target = computed<Target | null>(() =>
  tab.value?.kind === 'session' && session.value && session.value.state !== 'asleep'
    ? { kind: 'session', deck_id: tab.value.deck_id }
    : null,
);

const inGroup = computed(() => deck.selected.value?.kind === 'group');

function focus(): void {
  if (!focused.value) deck.focusPane(props.pane.id);
}
</script>

<template>
  <section
    class="flex h-full min-h-[160px] min-w-[320px] flex-1 flex-col overflow-hidden"
    :data-pane-id="pane.id"
    :data-focused="focused || undefined"
    aria-label="Pane"
    @mousedown.capture="focus"
    @focusin="focus"
  >
    <PaneTabs :pane="pane" :focused="focused" />
    <div class="relative min-h-0 flex-1 bg-deck-term" data-testid="pane-body">
      <ChangesTab v-if="tab?.kind === 'changes'" :key="'changes:' + tab.project_id" :project-id="tab.project_id" />
      <AsleepPanel v-else-if="tab?.kind === 'session' && session && !target" :key="'asleep:' + tab.deck_id" :deck-id="tab.deck_id" />
      <TerminalView v-else-if="tab?.kind === 'session' && target" :key="'session:' + tab.deck_id" :target="target" />
      <div
        v-else-if="inGroup"
        data-testid="group-empty"
        class="flex h-full flex-col items-center justify-center gap-3 p-6 text-center"
      >
        <LayoutGrid :size="40" :stroke-width="1.5" class="text-deck-faint" aria-hidden="true" />
        <h2 class="m-0 text-[15px] font-semibold text-deck-text">Empty group</h2>
        <p class="m-0 max-w-xs text-deck-muted">Drag sessions here from the sidebar, or right-click a session &rarr; Add to group</p>
      </div>
      <div v-else data-testid="pane-empty" class="flex h-full items-center justify-center p-6 text-center text-xs text-deck-faint">
        No open tabs
      </div>
      <DropOverlay :pane-id="pane.id" />
    </div>
  </section>
</template>
