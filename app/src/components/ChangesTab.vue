<script setup lang="ts">
import { AlertTriangle, ArrowRight, CheckCircle2, FileDiff, FolderGit2, RefreshCw } from 'lucide-vue-next';
import { computed, onMounted } from 'vue';
import type { Change, ChangeKind } from '../protocol';
import { useDeck } from '../useDeck';

const props = defineProps<{ projectId: string }>();
const deck = useDeck();

const GROUPS: { kind: ChangeKind; label: string; letter: string; badge: string }[] = [
  { kind: 'created', label: 'Created', letter: 'C', badge: 'bg-badge-c-bg text-badge-c-fg' },
  { kind: 'modified', label: 'Modified', letter: 'M', badge: 'bg-badge-m-bg text-badge-m-fg' },
  { kind: 'deleted', label: 'Deleted', letter: 'D', badge: 'bg-badge-d-bg text-badge-d-fg' },
  { kind: 'renamed', label: 'Renamed', letter: 'R', badge: 'bg-badge-r-bg text-badge-r-fg' },
];

const result = computed(() => deck.changes[props.projectId]);
const loading = computed(() => !result.value);
const projectName = computed(() => deck.projects.value.find((p) => p.id === props.projectId)?.name ?? '');
const list = computed(() => (result.value?.status === 'ok' ? result.value.list : null));

const groups = computed(() =>
  GROUPS.map((g) => ({ ...g, items: (list.value?.items ?? []).filter((c) => c.kind === g.kind) })).filter(
    (g) => g.items.length > 0,
  ),
);
const more = computed(() => (list.value ? Math.max(0, list.value.total - list.value.items.length) : 0));

function fileName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}
function folder(path: string): string {
  const i = path.lastIndexOf('/');
  return i < 0 ? '' : path.slice(0, i);
}
function isRename(c: Change): boolean {
  return c.kind === 'renamed' && !!c.old_path;
}

function refresh(): void {
  deck.refreshChanges(props.projectId);
}

onMounted(() => {
  if (!deck.changes[props.projectId]) refresh();
});
</script>

<template>
  <section class="flex h-full flex-col bg-deck-term text-[13px] text-deck-text">
    <header class="flex h-9 shrink-0 items-center gap-2 border-b border-deck-border bg-deck-panel pr-1.5 pl-4">
      <FileDiff :size="16" :stroke-width="1.75" class="shrink-0 text-deck-muted" aria-hidden="true" />
      <h2 class="flex min-w-0 flex-1 items-baseline gap-2">
        <span class="shrink-0 font-medium">Changes</span>
        <span class="truncate text-deck-muted">{{ projectName }}</span>
      </h2>
      <span
        v-if="list"
        data-testid="total"
        class="shrink-0 rounded-full bg-deck-hover px-2 text-xs leading-5 text-deck-muted"
        >{{ list.total }}</span
      >
      <button
        type="button"
        aria-label="Refresh changes"
        title="Refresh changes"
        class="flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none"
        @click="refresh"
      >
        <RefreshCw
          :size="16"
          :stroke-width="1.75"
          :class="loading ? 'animate-spin motion-reduce:animate-none' : ''"
          aria-hidden="true"
        />
      </button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto">
      <p v-if="loading" class="px-4 py-3 text-deck-muted">Loading…</p>
      <div v-else-if="result?.status === 'not_git'" class="flex flex-col items-center gap-1.5 px-6 py-12 text-center">
        <FolderGit2 :size="28" :stroke-width="1.5" class="text-deck-faint" aria-hidden="true" />
        <p class="mt-1 text-[15px] font-semibold">Not a git project</p>
        <p class="text-deck-muted">Initialise git in this folder to see changes here.</p>
      </div>
      <div v-else-if="result?.status === 'git_missing'" class="flex flex-col items-center gap-1.5 px-6 py-12 text-center">
        <AlertTriangle :size="28" :stroke-width="1.5" class="text-dot-needs" aria-hidden="true" />
        <p class="mt-1 text-[15px] font-semibold">Git isn't installed</p>
        <p class="text-deck-muted">Install git and refresh to see changes.</p>
      </div>
      <div v-else-if="result?.status === 'error'" role="alert" class="flex flex-col items-center gap-1.5 px-6 py-12 text-center text-deck-danger">
        <AlertTriangle :size="28" :stroke-width="1.5" aria-hidden="true" />
        <p>{{ result.message }}</p>
      </div>
      <div v-else-if="groups.length === 0" class="flex flex-col items-center gap-1.5 px-6 py-12 text-center">
        <CheckCircle2 :size="28" :stroke-width="1.5" class="text-dot-working" aria-hidden="true" />
        <p class="mt-1 text-[15px] font-semibold">No uncommitted changes</p>
        <p class="text-deck-muted">Your working tree is clean.</p>
      </div>
      <template v-else>
        <div v-for="g in groups" :key="g.kind" class="pt-3">
          <h3 class="mb-1 px-4 text-[11px] font-medium tracking-wider text-deck-faint uppercase">
            {{ g.label }} <span class="font-normal">{{ g.items.length }}</span>
          </h3>
          <ul>
            <li
              v-for="c in g.items"
              :key="c.path"
              class="flex h-[26px] cursor-default items-center gap-2 px-4 transition-colors duration-150 hover:bg-deck-hover"
            >
              <span
                data-testid="badge"
                class="flex size-4 shrink-0 items-center justify-center rounded font-mono text-[10px] leading-none font-bold"
                :class="g.badge"
                >{{ g.letter }}</span
              >
              <span class="shrink-0 font-mono text-[12.5px] text-deck-text">{{ fileName(c.path) }}</span>
              <span
                v-if="isRename(c)"
                class="flex min-w-0 items-center gap-1.5 font-mono text-xs text-deck-faint"
                :title="`${c.old_path} → ${c.path}`"
              >
                <span class="truncate">{{ c.old_path }}</span>
                <ArrowRight :size="12" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
                <span class="truncate">{{ c.path }}</span>
              </span>
              <span v-else class="min-w-0 truncate font-mono text-xs text-deck-faint" :title="c.path">{{ folder(c.path) }}</span>
              <span
                v-if="c.conflict"
                class="shrink-0 rounded-full bg-deck-danger/15 px-1.5 text-[10px] leading-4 text-deck-danger"
                >conflict</span
              >
            </li>
          </ul>
        </div>
        <p v-if="more > 0" class="px-4 py-3 text-xs text-deck-muted">and {{ more }} more</p>
      </template>
    </div>
  </section>
</template>
