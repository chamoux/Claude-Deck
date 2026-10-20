<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { FolderPlus } from 'lucide-vue-next';
import CloseDialog from './components/CloseDialog.vue';
import ConfirmDialog from './components/ConfirmDialog.vue';
import PastSessionsDialog from './components/PastSessionsDialog.vue';
import ProjectTree from './components/ProjectTree.vue';
import LayoutView from './components/LayoutView.vue';
import SettingsDialog from './components/SettingsDialog.vue';
import ShellPanel from './components/ShellPanel.vue';
import Splitter from './components/Splitter.vue';
import StatusFooter from './components/StatusFooter.vue';
import { dotClass } from './components/sessionDot';
import { pickAndAddProject } from './pickFolder';
import { allTabs, findPane, tabKey } from './layout';
import { useDeck } from './useDeck';

/**
 * Window shell: sidebar | split layout of the selected view (project or group) | project terminal, footer below.
 *
 * Keyboard shortcuts (documented decision):
 *  - Always global, even with focus in a terminal: Ctrl+Tab / Ctrl+Shift+Tab (next / previous tab in the
 *    focused pane), Ctrl+ (split the focused pane to the right), Ctrl+Alt+Arrows (move pane focus) and
 *    Ctrl+` (toggle the project terminal). A shell has no use for them.
 *  - Only when focus is OUTSIDE a terminal: Ctrl+T (new session), Ctrl+W (close the focused tab),
 *    Ctrl+1..9 (switch project). Inside a terminal these belong to the program (readline
 *    transpose-chars, delete-word, ...), so Ctrl+W deletes a word there and never closes a tab.
 *    Ctrl+C and every other key always go to the terminal.
 *  - No shortcut fires while a dialog is open.
 */
const deck = useDeck();

const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 360;
const SIDEBAR_DEFAULT = 256;

const sidebarWidth = ref(SIDEBAR_DEFAULT);
let sidebarDragStart: number | null = null;
function clampSidebar(w: number): number {
  return Math.round(Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, w)));
}
function onSidebarDrag(delta: number): void {
  sidebarDragStart ??= sidebarWidth.value;
  sidebarWidth.value = clampSidebar(sidebarDragStart + delta);
}

const settingsOpen = ref(false);
const pastFor = ref<string | null>(null);
const lastPastProject = ref('');
function openPast(projectId: string): void {
  lastPastProject.value = projectId;
  pastFor.value = projectId;
}

// Keep a valid project selected.
watch(
  deck.projects,
  (list) => {
    const sel = deck.selected.value;
    // A group selection is kept; only a missing selection or a vanished project falls back to the first project.
    if (list.length > 0 && (sel === null || (sel.kind === 'project' && !list.some((p) => p.id === sel.id)))) {
      deck.selectProject(list[0]!.id);
    }
  },
  { immediate: true },
);

const project = computed(() => deck.projects.value.find((p) => p.id === deck.selectedProjectId.value));
const hasView = computed(() => deck.selected.value !== null);
const viewName = computed(() => {
  const sel = deck.selected.value;
  if (sel?.kind === 'group') return deck.groups.value.find((g) => g.id === sel.id)?.name ?? 'Group';
  return project.value?.name ?? '';
});
// A project view without any tab keeps the "New Claude session" state; an empty group shows its own hint inside its pane.
const projectEmpty = computed(() => project.value !== undefined && allTabs(deck.currentLayout.value).length === 0);
const projectSessions = computed(() => (project.value ? deck.sessionsOf(project.value.id) : []));
const shellProjectId = deck.focusedProjectId;

// Measured width of the middle + right area, so the project terminal keeps 420 px for Claude.
const area = ref<HTMLElement | null>(null);
const areaWidth = ref(0);
let observer: ResizeObserver | null = null;
onMounted(() => {
  const el = area.value;
  if (!el) return;
  areaWidth.value = el.clientWidth;
  if (typeof ResizeObserver !== 'undefined') {
    observer = new ResizeObserver(() => {
      areaWidth.value = el.clientWidth;
    });
    observer.observe(el);
  }
});

// Window title + taskbar flash; flash only when the Needs you count rose.
let lastNeeds = 0;
watch(
  deck.windowTitle,
  (title) => {
    const n = deck.needsYouCount.value;
    const flash = n > lastNeeds;
    lastNeeds = n;
    void deck.attention(title, flash);
  },
  { immediate: true },
);

// Reconnect: the store bumps reattachGeneration; mounted TerminalViews re-attach themselves.

function focusedPane() {
  const id = deck.focusedPaneId.value;
  return id ? findPane(deck.currentLayout.value, id) : undefined;
}
function cycleTab(step: number): void {
  const pane = focusedPane();
  if (!pane || pane.tabs.length < 2) return;
  deck.setActiveTab(pane.id, (pane.active + step + pane.tabs.length * 2) % pane.tabs.length);
}
function closeFocusedTab(): void {
  const pane = focusedPane();
  const tab = pane?.tabs[pane.active];
  if (tab) deck.closeTab(tabKey(tab));
}
function toggleShellPanel(): void {
  const p = deck.projects.value.find((x) => x.id === shellProjectId.value);
  if (!p) return;
  deck.setShellPanel(p.id, { ...p.shell_panel, open: !p.shell_panel.open });
}

const ARROWS: Record<string, 'left' | 'right' | 'up' | 'down'> = {
  ArrowLeft: 'left',
  ArrowRight: 'right',
  ArrowUp: 'up',
  ArrowDown: 'down',
};

function inTerminal(e: KeyboardEvent): boolean {
  return e.target instanceof Element && e.target.closest('.xterm') !== null;
}
function onKeydown(e: KeyboardEvent): void {
  if (!e.ctrlKey || e.metaKey) return;
  if (deck.confirm.value || deck.closeDialogOpen.value || settingsOpen.value || pastFor.value) return;
  const handled = (): void => {
    e.preventDefault();
    e.stopPropagation();
  };
  if (e.altKey) {
    const dir = ARROWS[e.key];
    if (dir && !e.shiftKey) {
      handled();
      deck.moveFocus(dir);
    }
    return;
  }
  if (!e.shiftKey && (e.key === '\\' || e.code === 'Backslash')) {
    handled();
    deck.splitFocused('row');
    return;
  }
  if (e.key === 'Tab') {
    handled();
    cycleTab(e.shiftKey ? -1 : 1);
    return;
  }
  if (e.code === 'Backquote' || e.key === '`') {
    handled();
    toggleShellPanel();
    return;
  }
  if (inTerminal(e) || e.shiftKey) return;
  const pid = shellProjectId.value;
  const k = e.key.toLowerCase();
  if (k === 't' && pid) {
    handled();
    deck.newSession(pid);
  } else if (k === 'w') {
    handled();
    closeFocusedTab();
  } else if (/^[1-9]$/.test(k)) {
    const p = deck.projects.value[Number(k) - 1];
    if (p) {
      handled();
      deck.selectProject(p.id);
    }
  }
}
// Capture phase: runs before xterm sees the key, so the global shortcuts can win.
onMounted(() => window.addEventListener('keydown', onKeydown, true));
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown, true);
  observer?.disconnect();
});
</script>

<template>
  <div class="flex h-full flex-col bg-deck-bg font-sans text-[13px] text-deck-text">
    <div class="flex min-h-0 flex-1">
      <aside class="flex shrink-0 flex-col bg-deck-sidebar" :style="{ width: `${sidebarWidth}px` }">
        <div class="min-h-0 flex-1">
          <ProjectTree @resume-past="openPast" />
        </div>
      </aside>
      <Splitter
        label="Resize sidebar"
        @drag="onSidebarDrag"
        @end="sidebarDragStart = null"
        @reset="sidebarWidth = SIDEBAR_DEFAULT"
      />

      <div ref="area" class="flex min-w-0 flex-1">
        <template v-if="hasView">
          <section class="flex min-w-0 flex-1 flex-col" :aria-label="viewName">
            <div class="flex min-h-0 flex-1 bg-deck-term">
                <div v-if="projectEmpty" data-testid="no-tabs" class="flex h-full w-full flex-col items-center justify-center gap-3 p-6 text-center">
                  <FolderPlus :size="40" :stroke-width="1.5" class="text-deck-faint" aria-hidden="true" />
                  <h2 class="m-0 text-[15px] font-semibold text-deck-text">{{ project?.name }}</h2>
                  <p class="m-0 text-deck-muted">No open session tab. Start a new one or reopen an existing session.</p>
                  <button
                    type="button"
                    data-testid="empty-new-session"
                    class="h-8 cursor-pointer rounded-md bg-deck-accent px-3 text-[13px] font-medium text-white transition-colors duration-150 hover:bg-deck-accent-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:ring-offset-0"
                    @click="deck.newSession(project!.id)"
                  >
                    New Claude session
                  </button>
                  <ul v-if="projectSessions.length > 0" class="mt-2 flex w-full max-w-sm flex-col gap-0.5 text-left" aria-label="Sessions">
                    <li v-for="s in projectSessions" :key="s.deck_id">
                      <button
                        type="button"
                        :data-testid="'empty-session-' + s.deck_id"
                        :class="[
                          'flex h-[30px] w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
                          s.state === 'needs_you' ? 'bg-needs-soft text-dot-needs' : 'text-deck-text hover:bg-deck-hover',
                        ]"
                        @click="deck.openSession(s.deck_id)"
                      >
                        <span :class="['size-2 shrink-0 rounded-full', dotClass(s.state)]" aria-hidden="true" />
                        <span class="min-w-0 flex-1 truncate font-medium">{{ s.name || 'New session' }}</span>
                        <span v-if="s.state === 'needs_you'" class="text-xs font-semibold">Needs you</span>
                      </button>
                    </li>
                  </ul>
                  <p class="m-0 mt-1 font-mono text-xs text-deck-faint">Ctrl+T new session</p>
                </div>
              <LayoutView v-else :node="deck.currentLayout.value" :path="[]" />
            </div>
          </section>
          <ShellPanel v-if="shellProjectId" :project-id="shellProjectId" :container-width="areaWidth || undefined" />
        </template>
        <div v-else data-testid="no-projects" class="flex flex-1 flex-col items-center justify-center gap-3 bg-deck-term p-6 text-center">
          <FolderPlus :size="40" :stroke-width="1.5" class="text-deck-faint" aria-hidden="true" />
          <h2 class="m-0 text-[15px] font-semibold text-deck-text">Add a project folder to start</h2>
          <p class="m-0 text-deck-muted">Each project gets its own Claude sessions and a terminal.</p>
          <button
            type="button"
            data-testid="empty-add-project"
            class="h-8 cursor-pointer rounded-md bg-deck-accent px-3 text-[13px] font-medium text-white transition-colors duration-150 hover:bg-deck-accent-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:ring-offset-0"
            @click="pickAndAddProject(deck)"
          >
            Add project
          </button>
          <p class="m-0 mt-1 font-mono text-xs text-deck-faint">Ctrl+T new Claude session · Ctrl+` project terminal</p>
        </div>
      </div>
    </div>
    <StatusFooter @open-settings="settingsOpen = true" />

    <CloseDialog />
    <ConfirmDialog />
    <SettingsDialog :open="settingsOpen" @close="settingsOpen = false" />
    <PastSessionsDialog :project-id="lastPastProject" :open="pastFor !== null" @close="pastFor = null" />
  </div>
</template>
