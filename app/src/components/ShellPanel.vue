<script setup lang="ts">
import { PanelRightClose, SquareTerminal, X } from 'lucide-vue-next';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import type { ShellKind, Target } from '../protocol';
import { useDeck } from '../useDeck';
import Splitter from './Splitter.vue';
import TerminalView from './TerminalView.vue';

/**
 * Right-hand project terminal.
 * Props: projectId. `containerWidth` (optional) is the width in px of the row that holds the Claude
 * area plus this panel; the parent (App.vue) measures it. The panel never grows past
 * containerWidth - MIN_CLAUDE (420 px), so the Claude area keeps at least 420 px.
 * Without it only the absolute 1600 px cap applies.
 */
const props = defineProps<{ projectId: string; containerWidth?: number }>();

const MIN_WIDTH = 240;
const MAX_WIDTH = 1600;
const MIN_CLAUDE = 420;
const DEFAULT_WIDTH = 480;
const SAVE_DELAY_MS = 300;
const RAIL_WIDTH = 32;

const deck = useDeck();

const project = computed(() => deck.projects.value.find((p) => p.id === props.projectId));
const panel = computed(() => project.value?.shell_panel ?? { open: false, width: DEFAULT_WIDTH });
const target = computed<Target>(() => ({ kind: 'shell', project_id: props.projectId }));
const running = computed(() => deck.shellRunning(props.projectId));

const SHELL_NAMES: Record<ShellKind, string> = {
  power_shell: 'PowerShell',
  git_bash: 'Git Bash',
  cmd: 'cmd',
};
const shellName = computed(() => SHELL_NAMES[deck.state.value?.settings.shell ?? 'power_shell']);

const maxWidth = computed(() => {
  const room = props.containerWidth === undefined ? MAX_WIDTH : props.containerWidth - MIN_CLAUDE;
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, room));
});
function clamp(w: number): number {
  return Math.round(Math.min(maxWidth.value, Math.max(MIN_WIDTH, w)));
}

// Local width while dragging (and until the debounced save lands); null = follow the store.
const localWidth = ref<number | null>(null);
const width = computed(() => clamp(localWidth.value ?? panel.value.width));

let dragStart: number | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let pending: { projectId: string; width: number } | null = null;

function flush(): void {
  if (timer !== undefined) clearTimeout(timer);
  timer = undefined;
  const p = pending;
  pending = null;
  if (!p) return;
  const proj = deck.projects.value.find((x) => x.id === p.projectId);
  deck.setShellPanel(p.projectId, { open: proj?.shell_panel.open ?? true, width: p.width });
}

function scheduleSave(w: number): void {
  pending = { projectId: props.projectId, width: w };
  if (timer !== undefined) clearTimeout(timer);
  timer = setTimeout(flush, SAVE_DELAY_MS);
}

function onDrag(delta: number): void {
  dragStart ??= width.value;
  // Panel is on the right: moving the splitter left widens it.
  localWidth.value = clamp(dragStart - delta);
}
function onDragEnd(): void {
  dragStart = null;
  if (localWidth.value !== null) scheduleSave(localWidth.value);
}
function resetWidth(): void {
  localWidth.value = clamp(DEFAULT_WIDTH);
  scheduleSave(localWidth.value);
}

function setOpen(open: boolean): void {
  flush();
  deck.setShellPanel(props.projectId, { ...panel.value, width: width.value, open });
}

function askClose(): void {
  deck.confirm.value = {
    message: 'Close this terminal? Anything running in it stops.',
    yes: () => {
      deck.closeShell(props.projectId);
      // Hide the panel too: opening it again starts a new shell, using the current shell setting.
      setOpen(false);
    },
  };
}

// Switching project: save any pending width for the old one, then follow the new project's state.
watch(
  () => props.projectId,
  () => {
    flush();
    localWidth.value = null;
    dragStart = null;
  },
  { flush: 'sync' },
);
// Once the store reflects our saved width, drop the local override.
watch(
  () => panel.value.width,
  (w) => {
    if (pending === null && localWidth.value === w) localWidth.value = null;
  },
);
onBeforeUnmount(flush);
</script>

<template>
  <aside
    v-if="project && panel.open"
    data-testid="shell-open"
    class="flex h-full shrink-0 bg-deck-term"
    :style="{ width: `${width}px` }"
  >
    <Splitter label="Resize terminal" @drag="onDrag" @end="onDragEnd" @reset="resetWidth" />
    <div class="flex min-w-0 flex-1 flex-col">
      <header class="flex h-9 shrink-0 items-center gap-2 border-b border-deck-border bg-deck-panel pr-1.5 pl-3 text-[13px]">
        <SquareTerminal :size="16" :stroke-width="1.75" class="shrink-0 text-deck-muted" aria-hidden="true" />
        <span class="shrink-0 font-medium text-deck-text">Terminal</span>
        <span class="min-w-0 truncate text-deck-muted" data-testid="shell-project">{{ project.name }}</span>
        <span
          class="shrink-0 rounded border border-deck-border px-1.5 font-mono text-[11px] leading-5 text-deck-muted"
          data-testid="shell-kind"
          >{{ shellName }}</span
        >
        <span class="flex-1"></span>
        <button
          type="button"
          aria-label="Minimize terminal"
          title="Minimize terminal"
          class="flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
          @click="setOpen(false)"
        >
          <PanelRightClose :size="16" :stroke-width="1.75" aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label="Close terminal"
          title="Close terminal"
          class="flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
          @click="askClose"
        >
          <X :size="16" :stroke-width="1.75" aria-hidden="true" />
        </button>
      </header>
      <div class="min-h-0 flex-1">
        <TerminalView :key="projectId" :target="target" />
      </div>
    </div>
  </aside>
  <button
    v-else-if="project"
    type="button"
    data-testid="shell-rail"
    aria-label="Open terminal (Ctrl+`)"
    title="Open terminal (Ctrl+`)"
    class="flex h-full shrink-0 cursor-pointer flex-col items-center gap-3 border-l border-deck-border bg-deck-panel pt-3 text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none focus-visible:ring-inset"
    :style="{ width: `${RAIL_WIDTH}px` }"
    @click="setOpen(true)"
  >
    <span class="relative flex">
      <SquareTerminal :size="16" :stroke-width="1.75" aria-hidden="true" />
      <span
        v-if="running"
        data-testid="shell-running"
        class="absolute -top-1 -right-1 size-2 rounded-full bg-dot-working"
        title="Shell running"
      ></span>
    </span>
    <span class="text-xs [writing-mode:vertical-rl]">Terminal</span>
  </button>
</template>
