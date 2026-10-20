import type { SessionState } from '../protocol';

/** Tailwind classes for the state dot, shared by the sidebar row and the tab. */
export function dotClass(state: SessionState): string {
  switch (state) {
    case 'working':
      return 'bg-dot-working animate-pulse [animation-duration:2.5s] motion-reduce:animate-none';
    case 'idle':
      return 'bg-dot-idle';
    case 'needs_you':
      return 'bg-dot-needs';
    default:
      return 'border-2 border-dot-asleep bg-transparent';
  }
}
