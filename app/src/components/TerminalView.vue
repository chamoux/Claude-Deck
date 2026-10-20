<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { targetKey } from '../protocol';
import type { Target } from '../protocol';
import { useDeck } from '../useDeck';

const props = defineProps<{ target: Target }>();

const deck = useDeck();
const container = ref<HTMLDivElement | null>(null);

let term: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let observer: ResizeObserver | null = null;
let debounce: ReturnType<typeof setTimeout> | undefined;
let unsubscribe: (() => void) | null = null;
let current: Target = props.target;
let lastRows = 0;
let lastCols = 0;
let unmounted = false;

const FONT_FAMILY = '"JetBrains Mono Variable", "Cascadia Mono", Consolas, monospace';
const THEME = {
  background: '#13151a',
  foreground: '#c9ceda',
  cursor: '#7aa2f7',
  cursorAccent: '#13151a',
  selectionBackground: '#5b7cfa44',
  black: '#32344a',
  red: '#f7768e',
  green: '#9ece6a',
  yellow: '#e0af68',
  blue: '#7aa2f7',
  magenta: '#bb9af7',
  cyan: '#7dcfff',
  white: '#a9b1d6',
  brightBlack: '#565f89',
  brightRed: '#ff899d',
  brightGreen: '#b9f27c',
  brightYellow: '#ff9e64',
  brightBlue: '#8db0ff',
  brightMagenta: '#c9a9ff',
  brightCyan: '#a4daff',
  brightWhite: '#d5d9e8',
};

function subscribeAndAttach(target: Target): void {
  const t = term;
  if (!t) return;
  current = target;
  unsubscribe = deck.subscribeTerminal(target, {
    reset: () => t.reset(),
    write: (d) => t.write(d),
  });
  deck.attach(target, t.rows, t.cols);
  lastRows = t.rows;
  lastCols = t.cols;
}

function release(): void {
  deck.detach(current);
  unsubscribe?.();
  unsubscribe = null;
}

function refit(): void {
  if (!term || !fitAddon) return;
  fitAddon.fit();
  if (term.rows !== lastRows || term.cols !== lastCols) {
    lastRows = term.rows;
    lastCols = term.cols;
    deck.resize(current, term.rows, term.cols);
  }
}

function start(): void {
  const el = container.value;
  if (!el || unmounted) return;
  const t = new Terminal({
    fontFamily: FONT_FAMILY,
    fontSize: 13,
    lineHeight: 1.2,
    cursorBlink: true,
    scrollback: 5000,
    allowProposedApi: false,
    theme: THEME,
  });
  term = t;
  fitAddon = new FitAddon();
  t.loadAddon(fitAddon);
  t.open(el);
  fitAddon.fit();

  t.onData((s) => deck.input(current, new TextEncoder().encode(s)));
  t.onBinary((s) => deck.input(current, Uint8Array.from(s, (c) => c.charCodeAt(0) & 0xff)));

  subscribeAndAttach(props.target);

  observer = new ResizeObserver(() => {
    clearTimeout(debounce);
    debounce = setTimeout(refit, 100);
  });
  observer.observe(el);
}

onMounted(() => {
  // Measure cells with the real font: wait for it (jsdom has no document.fonts).
  const fonts = typeof document !== 'undefined' ? document.fonts : undefined;
  if (fonts?.load) {
    const timeout = new Promise<void>((resolve) => setTimeout(resolve, 1500));
    void Promise.race([fonts.load('13px "JetBrains Mono Variable"').then(() => undefined), timeout])
      .catch(() => undefined)
      .then(start);
  } else {
    start();
  }
});

watch(
  () => targetKey(props.target),
  () => {
    if (!term) return;
    release();
    term.reset();
    subscribeAndAttach(props.target);
  },
);

// After a reconnect the store forgot every terminal index: attach again for a fresh snapshot.
// Our sink stays subscribed; the Attached event resets it before the snapshot arrives.
watch(deck.reattachGeneration, () => {
  if (!term) return;
  deck.attach(current, term.rows, term.cols);
  lastRows = term.rows;
  lastCols = term.cols;
});

onBeforeUnmount(() => {
  unmounted = true;
  clearTimeout(debounce);
  observer?.disconnect();
  observer = null;
  if (term) release();
  term?.dispose();
  term = null;
  fitAddon = null;
});

defineExpose({ focus: () => term?.focus() });
</script>

<template>
  <!-- Padding lives on the wrapper so xterm's fit measures the inner box. -->
  <div class="box-border h-full w-full bg-deck-term p-3" data-testid="terminal-pad">
    <div ref="container" class="h-full w-full"></div>
  </div>
</template>
