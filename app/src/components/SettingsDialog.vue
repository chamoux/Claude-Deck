<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { ChevronDown, FolderOpen } from 'lucide-vue-next';
import type { ShellKind } from '../protocol';
import { useDeck } from '../useDeck';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();

const MIN = 5;
const MAX = 240;
const DEFAULT_MINUTES = 30;

const deck = useDeck();
const panel = ref<HTMLElement | null>(null);
const off = ref(true);
const minutes = ref<number | string>(DEFAULT_MINUTES);
const path = ref('');
const shell = ref<ShellKind>('power_shell');

const minutesValid = computed(() => {
  const n = Number(minutes.value);
  return minutes.value !== '' && Number.isInteger(n) && n >= MIN && n <= MAX;
});
const canSave = computed(() => off.value || minutesValid.value);

function load(): void {
  const s = deck.state.value?.settings;
  off.value = s?.auto_sleep_minutes == null;
  minutes.value = s?.auto_sleep_minutes ?? DEFAULT_MINUTES;
  path.value = s?.claude_path ?? '';
  shell.value = s?.shell ?? 'power_shell';
}

watch(
  () => props.open,
  async (open) => {
    if (!open) return;
    load();
    await nextTick();
    panel.value?.querySelector<HTMLElement>('#set-minutes, #set-off')?.focus();
  },
  { immediate: true, flush: 'post' },
);

async function browse(): Promise<void> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const picked = await open({ filters: [{ name: 'claude', extensions: ['exe'] }] });
  if (typeof picked === 'string') path.value = picked;
}

function save(): void {
  if (!canSave.value) return;
  const current = deck.state.value?.settings;
  if (!current) return;
  deck.setSettings({
    ...current,
    auto_sleep_minutes: off.value ? null : Number(minutes.value),
    claude_path: path.value.trim() || null,
    shell: shell.value,
  });
  emit('close');
}

function trap(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    e.preventDefault();
    emit('close');
    return;
  }
  if (e.key !== 'Tab' || !panel.value) return;
  const f = Array.from(
    panel.value.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled])'),
  );
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

const field =
  'h-8 rounded-md border border-deck-border-strong bg-deck-bg px-2.5 text-deck-text transition-colors duration-150 placeholder:text-deck-faint focus:border-deck-accent focus:outline-none';
const btnRing = 'focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:outline-none';
</script>

<template>
  <Transition
    enter-active-class="transition-opacity duration-150 motion-reduce:transition-none"
    enter-from-class="opacity-0 *:scale-95 motion-reduce:*:scale-100"
  >
    <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center bg-black/60" @keydown="trap">
      <div
        ref="panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-dialog-title"
        class="w-120 max-w-[90vw] rounded-xl border border-deck-border-strong bg-deck-elevated p-5 text-deck-text shadow-2xl transition-transform duration-150 motion-reduce:transition-none"
      >
        <h2 id="settings-dialog-title" class="text-[15px] font-semibold">Settings</h2>

        <div class="mt-4 flex flex-col gap-4">
          <div class="flex flex-col gap-1">
            <div class="flex items-center justify-between gap-3">
              <label for="set-off" class="cursor-pointer text-[13px] font-medium">Auto-sleep</label>
              <button
                id="set-off"
                type="button"
                role="switch"
                :aria-checked="!off"
                aria-describedby="set-sleep-help"
                class="relative h-5 w-9 shrink-0 cursor-pointer rounded-full transition-colors duration-150 motion-reduce:transition-none"
                :class="[btnRing, off ? 'bg-deck-border-strong' : 'bg-deck-accent']"
                @click="off = !off"
              >
                <span
                  class="absolute top-0.5 left-0.5 size-4 rounded-full bg-white transition-transform duration-150 motion-reduce:transition-none"
                  :class="off ? 'translate-x-0' : 'translate-x-4'"
                />
              </button>
            </div>
            <p id="set-sleep-help" class="text-xs text-deck-muted">
              Idle sessions are put to sleep to save memory. Working sessions never sleep.
            </p>
            <template v-if="!off">
              <div class="mt-1.5 flex items-center gap-2">
                <input
                  id="set-minutes"
                  v-model="minutes"
                  type="number"
                  inputmode="numeric"
                  :min="MIN"
                  :max="MAX"
                  aria-label="Minutes before sleep"
                  :aria-invalid="!minutesValid"
                  aria-describedby="set-minutes-err"
                  :class="[
                    field,
                    'w-16 text-center [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none',
                    !minutesValid && 'border-deck-danger',
                  ]"
                />
                <span class="text-xs text-deck-muted">minutes</span>
              </div>
              <p id="set-minutes-err" class="text-xs text-deck-danger" role="alert">
                <template v-if="!minutesValid">Enter a whole number from {{ MIN }} to {{ MAX }}.</template>
              </p>
            </template>
          </div>

          <div class="flex flex-col gap-1">
            <label for="set-path" class="text-[13px] font-medium">Claude executable</label>
            <p id="set-path-help" class="text-xs text-deck-muted">Leave empty to find claude.exe automatically.</p>
            <div class="mt-1.5 flex gap-2">
              <input
                id="set-path"
                v-model="path"
                type="text"
                spellcheck="false"
                placeholder="Auto-detect"
                aria-describedby="set-path-help"
                :class="[field, 'min-w-0 flex-1 font-mono text-xs']"
              />
              <button
                type="button"
                class="flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-deck-border-strong px-3 whitespace-nowrap transition-colors duration-150 hover:bg-deck-hover"
                :class="btnRing"
                @click="browse"
              >
                <FolderOpen :size="14" :stroke-width="1.75" aria-hidden="true" />
                Browse
              </button>
            </div>
          </div>

          <div class="flex flex-col gap-1">
            <label for="set-shell" class="text-[13px] font-medium">Project terminal</label>
            <p id="set-shell-help" class="text-xs text-deck-muted">The shell opened in each project's terminal panel.</p>
            <div class="relative mt-1.5">
              <select
                id="set-shell"
                v-model="shell"
                aria-describedby="set-shell-help"
                :class="[field, 'w-full cursor-pointer appearance-none pr-8']"
              >
                <option value="power_shell">PowerShell</option>
                <option value="git_bash">Git Bash</option>
                <option value="cmd">cmd</option>
              </select>
              <ChevronDown
                :size="14"
                :stroke-width="1.75"
                class="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 text-deck-faint"
                aria-hidden="true"
              />
            </div>
          </div>
        </div>

        <div class="mt-5 flex justify-end gap-2">
          <button
            type="button"
            class="h-8 cursor-pointer rounded-md px-3 whitespace-nowrap text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text"
            :class="btnRing"
            @click="emit('close')"
          >
            Cancel
          </button>
          <button
            type="button"
            :disabled="!canSave"
            class="h-8 cursor-pointer rounded-md bg-deck-accent px-3 font-medium whitespace-nowrap text-white transition-colors duration-150 hover:bg-deck-accent-hover focus-visible:ring-2 focus-visible:ring-white/80 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-40"
            @click="save"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
