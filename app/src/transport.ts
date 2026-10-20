import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Event, Request } from './protocol';

export type Connection = 'connected' | 'reconnecting' | 'failed';

export interface Transport {
  send(req: Request): Promise<void>;
  input(index: number, data: Uint8Array): Promise<void>;
  onEvent(cb: (e: Event) => void): () => void;
  onOutput(cb: (index: number, data: Uint8Array) => void): () => void;
  onConnection(cb: (s: Connection) => void): () => void;
  onCloseRequested(cb: () => void): () => void;
  close(keepRunning: boolean): Promise<void>;
  attention(title: string, flash: boolean): Promise<void>;
}

interface OutputChunk {
  index: number;
  data: number[];
}

/** Subscribes to a Tauri event; returns a synchronous unsubscribe even though listen() is async. */
function listenSync<T>(name: string, cb: (payload: T) => void): () => void {
  let cancelled = false;
  let unlisten: (() => void) | null = null;
  void listen<T>(name, (e) => cb(e.payload)).then((fn) => {
    if (cancelled) fn();
    else unlisten = fn;
  });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

export function tauriTransport(): Transport {
  const outputListeners = new Set<(index: number, data: Uint8Array) => void>();
  let subscribed = false;

  function ensureSubscribed(): void {
    if (subscribed) return;
    subscribed = true;
    const channel = new Channel<OutputChunk>();
    channel.onmessage = (chunk) => {
      const data = Uint8Array.from(chunk.data);
      for (const cb of outputListeners) cb(chunk.index, data);
    };
    void invoke('deck_subscribe', { output: channel });
  }

  return {
    send: (req) => invoke('deck_send', { json: JSON.stringify(req) }),
    input: (index, data) => invoke('deck_input', { index, data: Array.from(data) }),
    onEvent(cb) {
      return listenSync<string>('deck:event', (json) => cb(JSON.parse(json) as Event));
    },
    onOutput(cb) {
      outputListeners.add(cb);
      ensureSubscribed();
      return () => {
        outputListeners.delete(cb);
      };
    },
    onConnection: (cb) => listenSync<Connection>('deck:connection', cb),
    onCloseRequested: (cb) => listenSync<unknown>('deck:close-requested', () => cb()),
    close: (keepRunning) => invoke('deck_close', { keepRunning }),
    attention: (title, flash) => invoke('deck_attention', { title, flash }),
  };
}

export interface FakeTransport extends Transport {
  sent: Request[];
  inputs: { index: number; data: Uint8Array }[];
  closed: boolean[];
  attentions: { title: string; flash: boolean }[];
  emit(e: Event): void;
  emitOutput(index: number, data: Uint8Array): void;
  emitConnection(s: Connection): void;
  emitCloseRequested(): void;
}

export function fakeTransport(): FakeTransport {
  const events = new Set<(e: Event) => void>();
  const outputs = new Set<(index: number, data: Uint8Array) => void>();
  const connections = new Set<(s: Connection) => void>();
  const closeReqs = new Set<() => void>();
  const sub = <T>(set: Set<T>, cb: T): (() => void) => {
    set.add(cb);
    return () => {
      set.delete(cb);
    };
  };
  const t: FakeTransport = {
    sent: [],
    inputs: [],
    closed: [],
    attentions: [],
    send(req) {
      t.sent.push(req);
      return Promise.resolve();
    },
    input(index, data) {
      t.inputs.push({ index, data });
      return Promise.resolve();
    },
    onEvent: (cb) => sub(events, cb),
    onOutput: (cb) => sub(outputs, cb),
    onConnection: (cb) => sub(connections, cb),
    onCloseRequested: (cb) => sub(closeReqs, cb),
    close(keepRunning) {
      t.closed.push(keepRunning);
      return Promise.resolve();
    },
    attention(title, flash) {
      t.attentions.push({ title, flash });
      return Promise.resolve();
    },
    emit: (e) => events.forEach((cb) => cb(e)),
    emitOutput: (index, data) => outputs.forEach((cb) => cb(index, data)),
    emitConnection: (s) => connections.forEach((cb) => cb(s)),
    emitCloseRequested: () => closeReqs.forEach((cb) => cb()),
  };
  return t;
}
