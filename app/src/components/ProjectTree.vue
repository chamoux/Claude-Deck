<script setup lang="ts">
import {
  ArrowDown,
  ArrowUp,
  ChevronRight,
  Folder,
  FolderOpen,
  FolderX,
  History,
  LayoutGrid,
  Moon,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from 'lucide-vue-next';
import { computed, reactive, ref } from 'vue';
import { panes } from '../layout';
import { pickAndAddProject } from '../pickFolder';
import { useDeck } from '../useDeck';
import ContextMenu from './ContextMenu.vue';
import type { MenuItem } from './ContextMenu.vue';
import GroupList from './GroupList.vue';
import SessionRow from './SessionRow.vue';

const PROJECT_MIME = 'application/x-deck-project';

const emit = defineEmits<{ 'resume-past': [projectId: string] }>();

const deck = useDeck();
const root = ref<HTMLElement | null>(null);
const collapsed = reactive<Record<string, boolean>>({});
const renamingId = ref<string | null>(null);

type MenuTarget = { x: number; y: number; kind: 'project' | 'session'; id: string };
const menu = ref<MenuTarget | null>(null);

const missing = computed(() => new Set(deck.state.value?.missing_folders ?? []));

const query = ref('');
const filterInput = ref<HTMLInputElement | null>(null);
/** Projects whose name contains the filter text (any case). */
const visibleProjects = computed(() => {
  const q = query.value.trim().toLocaleLowerCase();
  if (!q) return deck.projects.value;
  return deck.projects.value.filter((p) => p.name.toLocaleLowerCase().includes(q));
});

function clearFilter(): void {
  query.value = '';
  filterInput.value?.focus();
}

function onFilterEscape(): void {
  if (query.value) query.value = '';
  else filterInput.value?.blur();
}

function openMenu(e: MouseEvent, kind: MenuTarget['kind'], id: string): void {
  menu.value = { x: e.clientX, y: e.clientY, kind, id };
}

const menuItems = computed<MenuItem[]>(() => {
  const m = menu.value;
  if (!m) return [];
  if (m.kind === 'project') {
    const i = deck.projects.value.findIndex((p) => p.id === m.id);
    return [
      { key: 'new', label: 'New Claude session', icon: Plus },
      { key: 'resume', label: 'Resume past session…', icon: History },
      { key: 'explorer', label: 'Open in File Explorer', icon: FolderOpen, separator: true },
      { key: 'move-up', label: 'Move up', icon: ArrowUp, disabled: i <= 0, separator: true },
      { key: 'move-down', label: 'Move down', icon: ArrowDown, disabled: i === deck.projects.value.length - 1 },
      { key: 'remove', label: 'Remove from list', icon: Trash2, danger: true, separator: true },
    ];
  }
  const asleep = deck.session(m.id)?.state === 'asleep';
  const groupItems: MenuItem[] = deck.groups.value.map((g) => ({ key: 'group:' + g.id, label: g.name, icon: LayoutGrid }));
  groupItems.push({ key: 'new-group', label: 'New group…', icon: Plus, separator: groupItems.length > 0 });
  return [
    { key: 'rename', label: 'Rename', icon: Pencil },
    { key: 'add-to-group', label: 'Add to group', icon: LayoutGrid, children: groupItems },
    { key: 'sleep', label: 'Sleep now', icon: Moon, disabled: asleep },
    { key: 'close', label: 'Close session', icon: X, separator: true },
  ];
});

async function onSelect(key: string): Promise<void> {
  const m = menu.value;
  if (!m) return;
  if (key.startsWith('group:')) {
    deck.addToGroup(key.slice('group:'.length), m.id);
    return;
  }
  switch (key) {
    case 'new-group':
      deck.newGroup(undefined, { addSession: m.id });
      break;
    case 'new':
      deck.newSession(m.id);
      break;
    case 'resume':
      emit('resume-past', m.id);
      break;
    case 'explorer': {
      const path = deck.projects.value.find((p) => p.id === m.id)?.path;
      if (path) {
        const { openPath } = await import('@tauri-apps/plugin-opener');
        await openPath(path);
      }
      break;
    }
    case 'move-up':
    case 'move-down': {
      const i = deck.projects.value.findIndex((p) => p.id === m.id);
      deck.moveProject(m.id, i + (key === 'move-up' ? -1 : 1));
      break;
    }
    case 'remove':
      deck.removeProject(m.id);
      break;
    case 'rename':
      renamingId.value = m.id;
      break;
    case 'sleep':
      deck.sleep(m.id);
      break;
    case 'close':
      deck.closeSession(m.id);
      break;
  }
}

function toggleProject(id: string): void {
  deck.selectProject(id);
  collapsed[id] = !collapsed[id];
}

function addProject(): void {
  void pickAndAddProject(deck);
}

function saveRename(deckId: string, name: string): void {
  deck.rename(deckId, name);
  renamingId.value = null;
}

/** The focused pane's active tab in the current view. */
const activeSessionId = computed<string | null>(() => {
  const layout = deck.currentLayout.value;
  const list = panes(layout);
  const pane = list.find((p) => p.id === deck.focusedPaneId.value) ?? list[0];
  const t = pane?.tabs[pane.active];
  return t?.kind === 'session' ? t.deck_id : null;
});

function isProjectSelected(id: string): boolean {
  const s = deck.selected.value;
  return s?.kind === 'project' && s.id === id;
}

// ---- Reorder by drag and drop --------------------------------------------------------------
// dataTransfer is unreadable during dragover, so the dragged project is kept in `draggingId`.

const draggingId = ref<string | null>(null);
/** Where the dragged project would land: before or after this project. */
const dropAt = ref<{ id: string; after: boolean } | null>(null);

function onProjectDragStart(e: DragEvent, id: string): void {
  if (!e.dataTransfer) return;
  e.dataTransfer.setData(PROJECT_MIME, id);
  e.dataTransfer.effectAllowed = 'move';
  draggingId.value = id;
}

function endProjectDrag(): void {
  draggingId.value = null;
  dropAt.value = null;
}

function onProjectDragOver(e: DragEvent, id: string): void {
  if (!draggingId.value || !Array.from(e.dataTransfer?.types ?? []).includes(PROJECT_MIME)) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  if (id === draggingId.value) {
    dropAt.value = null;
    return;
  }
  // Upper half of the project row: before it. Lower half, or over its sessions: after it.
  const row = (e.currentTarget as HTMLElement).querySelector('[data-project-row]')?.getBoundingClientRect();
  dropAt.value = { id, after: !row || e.clientY > row.top + row.height / 2 };
}

function onProjectDragLeave(e: DragEvent, id: string): void {
  const li = e.currentTarget as HTMLElement;
  if (dropAt.value?.id === id && !li.contains(e.relatedTarget as Node | null)) dropAt.value = null;
}

function onProjectDrop(e: DragEvent): void {
  const id = draggingId.value;
  const at = dropAt.value;
  if (!id) return;
  e.preventDefault();
  endProjectDrag();
  if (!at) return;
  const ids = deck.projects.value.map((p) => p.id);
  const from = ids.indexOf(id);
  let to = ids.indexOf(at.id) + (at.after ? 1 : 0);
  if (from < to) to--;
  deck.moveProject(id, to);
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
  const rows = Array.from(root.value?.querySelectorAll<HTMLElement>('[data-nav]') ?? []);
  if (rows.length === 0) return;
  e.preventDefault();
  const i = rows.indexOf(document.activeElement as HTMLElement);
  const delta = e.key === 'ArrowDown' ? 1 : -1;
  const next = i === -1 ? (delta > 0 ? 0 : rows.length - 1) : Math.min(rows.length - 1, Math.max(0, i + delta));
  rows[next].focus();
}
</script>

<template>
  <nav
    ref="root"
    aria-label="Projects"
    class="flex h-full w-full flex-col overflow-y-auto border-r border-deck-border bg-deck-sidebar font-sans text-[13px] text-deck-text select-none"
    @keydown="onKeydown"
  >
    <GroupList />

    <header class="flex items-center justify-between py-2 pr-2 pl-3">
      <h2 class="text-[11px] font-semibold tracking-wider text-deck-faint uppercase">PROJECTS</h2>
      <button
        type="button"
        data-testid="add-project"
        aria-label="Add project folder"
        title="Add project folder"
        class="flex size-7 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
        @click="addProject"
      >
        <Plus :size="16" :stroke-width="1.75" aria-hidden="true" />
      </button>
    </header>

    <div v-if="deck.projects.value.length > 0" class="relative px-1.5 pb-1.5">
      <Search
        :size="14"
        :stroke-width="1.75"
        class="pointer-events-none absolute top-[calc(50%-3px)] left-4 -translate-y-1/2 text-deck-faint"
        aria-hidden="true"
      />
      <input
        ref="filterInput"
        v-model="query"
        type="text"
        data-testid="project-filter"
        aria-label="Filter projects by name"
        placeholder="Filter projects"
        spellcheck="false"
        autocomplete="off"
        class="h-7 w-full rounded-md border border-deck-border-strong bg-deck-bg pr-7 pl-7 font-sans text-[13px] text-deck-text transition-colors duration-150 placeholder:text-deck-faint focus:border-deck-accent focus:outline-none"
        @keydown.esc.prevent="onFilterEscape"
      />
      <button
        v-if="query"
        type="button"
        data-testid="clear-filter"
        aria-label="Clear filter"
        title="Clear filter"
        class="absolute top-[calc(50%-3px)] right-2.5 flex size-5 -translate-y-1/2 cursor-pointer items-center justify-center rounded text-deck-faint transition-colors duration-150 hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
        @click="clearFilter"
      >
        <X :size="14" :stroke-width="1.75" aria-hidden="true" />
      </button>
    </div>

    <p v-if="query.trim() && visibleProjects.length === 0" data-testid="no-match" class="px-3 pb-3 text-xs text-deck-faint">
      No projects match “{{ query.trim() }}”
    </p>

    <ul class="flex flex-col gap-0.5 px-1.5 pb-3">
      <li
        v-for="p in visibleProjects"
        :key="p.id"
        :data-testid="'project-' + p.id"
        class="relative"
        @dragenter="onProjectDragOver($event, p.id)"
        @dragover="onProjectDragOver($event, p.id)"
        @dragleave="onProjectDragLeave($event, p.id)"
        @drop="onProjectDrop"
      >
        <span
          v-if="dropAt?.id === p.id"
          data-testid="drop-line"
          :class="[
            'pointer-events-none absolute inset-x-1 z-10 h-0.5 rounded-full bg-deck-accent',
            dropAt?.after ? '-bottom-[2px]' : '-top-[2px]',
          ]"
          aria-hidden="true"
        />
        <button
          type="button"
          data-nav
          data-project-row
          draggable="true"
          :aria-expanded="!collapsed[p.id]"
          :title="p.path"
          :class="[
            'flex h-[30px] w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left font-medium transition-colors duration-150 hover:bg-deck-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
            isProjectSelected(p.id) ? 'text-deck-text' : 'text-deck-muted',
            missing.has(p.id) || draggingId === p.id ? 'opacity-50' : '',
          ]"
          @click="toggleProject(p.id)"
          @contextmenu.prevent="openMenu($event, 'project', p.id)"
          @dragstart="onProjectDragStart($event, p.id)"
          @dragend="endProjectDrag"
        >
          <ChevronRight
            :size="14"
            :stroke-width="1.75"
            class="shrink-0 text-deck-faint transition-transform duration-150"
            :class="collapsed[p.id] ? '' : 'rotate-90'"
            aria-hidden="true"
          />
          <FolderX v-if="missing.has(p.id)" :size="14" :stroke-width="1.75" class="shrink-0 text-deck-danger" aria-hidden="true" />
          <FolderOpen v-else-if="!collapsed[p.id]" :size="14" :stroke-width="1.75" class="shrink-0 text-deck-faint" aria-hidden="true" />
          <Folder v-else :size="14" :stroke-width="1.75" class="shrink-0 text-deck-faint" aria-hidden="true" />
          <span class="min-w-[4.5rem] flex-1 truncate" :title="p.path">{{ p.name }}</span>
          <span v-if="missing.has(p.id)" class="shrink-0 text-[11px] font-normal text-deck-danger">Folder missing</span>
          <span
            v-if="(deck.needsYouByProject.value[p.id] ?? 0) > 0"
            data-testid="needs-badge"
            class="shrink-0 rounded-full bg-dot-needs/20 px-1.5 text-[11px] leading-4 font-semibold text-dot-needs"
            >{{ deck.needsYouByProject.value[p.id] }}</span
          >
          <span data-testid="count" class="min-w-4 shrink-0 text-right font-mono text-xs font-normal text-deck-faint">{{
            deck.sessionsOf(p.id).length
          }}</span>
        </button>

        <ul v-if="!collapsed[p.id]" class="mt-0.5 flex flex-col gap-0.5">
          <li v-for="s in deck.sessionsOf(p.id)" :key="s.deck_id" :data-testid="'session-' + s.deck_id">
            <SessionRow
              :session="s"
              :active="activeSessionId === s.deck_id"
              :renaming="renamingId === s.deck_id"
              @open="deck.openSession(s.deck_id)"
              @menu="openMenu($event, 'session', s.deck_id)"
              @rename="saveRename(s.deck_id, $event)"
              @cancel-rename="renamingId = null"
            />
          </li>
        </ul>
      </li>
    </ul>

    <ContextMenu
      v-if="menu"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @select="onSelect"
      @close="menu = null"
    />
  </nav>
</template>
