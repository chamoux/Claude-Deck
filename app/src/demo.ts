// Browser-only demo transport for design review. Loaded via dynamic import from main.ts in DEV only.
import type { LayoutNode } from './layout';
import type { Event, Request, SessionView, StateView, Target } from './protocol';
import { fakeTransport } from './transport';
import type { FakeTransport } from './transport';
import type { Deck } from './useDeck';

const enc = new TextEncoder();
const ESC = '\x1b[';
const c = (code: string, s: string): string => `${ESC}${code}m${s}${ESC}0m`;

const HOME = 'C:\\Users\\Chamoux\\Desktop';
const P = { codehive: 'p-codehive', netwatch: 'p-netwatch', diorama: 'p-diorama', saw: 'p-saw' };
const S = {
  fix: 'd-fix',
  qa: 'd-qa',
  export: 'd-export',
  long: 'd-long',
  nwIdle: 'd-nw-idle',
  nwAsleep: 'd-nw-asleep',
  dio: 'd-dio',
};

function session(
  deck_id: string,
  project_id: string,
  name: string | null,
  state: SessionView['state'],
  error: string | null = null,
): SessionView {
  return { deck_id, project_id, name, state, background_work: false, error };
}

const paneOf = (id: string, ...ids: string[]): LayoutNode => ({
  type: 'pane',
  id,
  tabs: ids.map((deck_id) => ({ kind: 'session', deck_id })),
  active: 0,
});
const splitOf = (dir: 'row' | 'column', a: LayoutNode, b: LayoutNode): LayoutNode => ({ type: 'split', dir, ratio: 0.5, a, b });
const G_BILLING = 'g-billing';

function demoState(): StateView {
  const panel = (open: boolean) => ({ open, width: 480 });
  return {
    projects: [
      { id: P.codehive, path: `${HOME}\\CodeHive`, name: 'CodeHive', shell_panel: panel(true) },
      { id: P.netwatch, path: `${HOME}\\NetWatch`, name: 'NetWatch', shell_panel: panel(false) },
      { id: P.diorama, path: `${HOME}\\diorama`, name: 'diorama', shell_panel: panel(false) },
      { id: P.saw, path: `${HOME}\\SawlyPOS`, name: 'SawlyPOS', shell_panel: panel(false) },
    ],
    sessions: [
      session(S.fix, P.codehive, 'fix invoice discount applied twice', 'working'),
      session(S.qa, P.codehive, 'QA guest bugs', 'needs_you'),
      session(S.export, P.codehive, 'add export button', 'idle'),
      session(
        S.long,
        P.codehive,
        'investigate why the nightly reconciliation job times out when the ledger has more than ten thousand open entries',
        'asleep',
      ),
      session(S.nwIdle, P.netwatch, 'build plan', 'idle'),
      session(S.nwAsleep, P.netwatch, 'dns leak check', 'asleep', "Can't resume"),
      session(S.dio, P.diorama, 'scene lighting pass', 'idle'),
    ],
    shells: [{ project_id: P.codehive, running: true }],
    settings: { auto_sleep_minutes: 30, claude_path: null, window: null, shell: 'power_shell' },
    missing_folders: [P.saw],
    groups: [],
  };
}

function claudeScreen(): string {
  const l = (s: string) => s + '\r\n';
  return (
    l(c('38;5;208', '╭────────────────────────────────────────────────╮')) +
    l(c('38;5;208', '│ ') + c('1;38;5;208', '✻ Welcome to Claude Code') + c('38;5;208', '                       │')) +
    l(c('38;5;208', '│ ') + c('2', '/help for help, /status for your setup') + c('38;5;208', '    │')) +
    l(c('38;5;208', '╰────────────────────────────────────────────────╯')) +
    l('') +
    l(c('1', '> fix the invoice total')) +
    l('') +
    l(c('32', '●') + ' Read(' + c('36', 'app/Services/InvoiceService.php') + ')') +
    l('  ⎿  ' + c('2', 'Read 214 lines')) +
    l(c('32', '●') + ' Update(' + c('36', 'app/Services/InvoiceService.php') + ')') +
    l('  ⎿  ' + c('2', 'Updated with ') + c('32', '4 additions') + c('2', ' and ') + c('31', '2 removals')) +
    l(c('33', '●') + ' Bash(' + c('36', 'php artisan test --filter=Invoice') + ')') +
    l('  ⎿  ' + c('2', 'Running…')) +
    l('') +
    l(c('38;5;208', '✻ Pondering…') + c('2', ' (12s · esc to interrupt)')) +
    l('') +
    l('╭──────────────────────────────────────────────────────╮') +
    l('│ > ' + c('2', 'Try "write a test for InvoiceService"') + '                  │') +
    l('╰──────────────────────────────────────────────────────╯') +
    c('2', '  ? for shortcuts                       Context left: 71%')
  );
}

function shellScreen(): string {
  const l = (s: string) => s + '\r\n';
  const prompt = c('32', 'PS ') + 'C:\\Users\\Chamoux\\Desktop\\CodeHive> ';
  return (
    l('Windows PowerShell') +
    l('') +
    l(prompt + 'git status') +
    l('On branch main') +
    l("Your branch is up to date with 'origin/main'.") +
    l('') +
    l('Changes not staged for commit:') +
    l(c('2', '  (use "git add <file>..." to update what will be committed)')) +
    l('        ' + c('31', 'modified:   app/Services/InvoiceService.php')) +
    l('        ' + c('31', 'modified:   tests/Feature/InvoiceTest.php')) +
    l('') +
    l('Untracked files:') +
    l('        ' + c('31', 'resources/js/Pages/Export.vue')) +
    l('') +
    l('Changes to be committed:') +
    l('        ' + c('32', 'new file:   app/Http/Controllers/ExportController.php')) +
    l('') +
    prompt
  );
}

export interface DemoTransport extends FakeTransport {
  /** Waits for the demo State, then drives the scene through the store's public actions. */
  runScene(deck: Deck, scene: string | null): Promise<void>;
}

export function createDemoTransport(): DemoTransport {
  const t = fakeTransport() as DemoTransport;
  let state = demoState();
  let nextIndex = 1;
  let memTimer: ReturnType<typeof setInterval> | undefined;
  let loaded: () => void = () => {};
  const loadedP = new Promise<void>((res) => (loaded = res));
  const later = (fn: () => void, ms = 0) => setTimeout(fn, ms);
  const emitState = () => t.emit({ type: 'state', state: structuredClone(state) });

  const baseSend = t.send;
  t.send = (req: Request) => {
    void baseSend(req);
    handle(req);
    return Promise.resolve();
  };

  function handle(req: Request): void {
    switch (req.type) {
      case 'hello':
        later(() => {
          emitState();
          loaded();
          t.emit({ type: 'memory', awake: 3, bytes: 1932735283 });
          memTimer ??= setInterval(() => t.emit({ type: 'memory', awake: 3, bytes: 1932735283 }), 5000);
        });
        break;
      case 'attach': {
        const index = nextIndex++;
        const target: Target = req.target;
        later(() => {
          t.emit({ type: 'attached', target, index });
          t.emitOutput(index, enc.encode(target.kind === 'session' ? claudeScreen() : shellScreen()));
        }, 20);
        break;
      }
      case 'git_status':
        later(() =>
          t.emit({
            type: 'git_status',
            project_id: req.project_id,
            result: {
              status: 'ok',
              list: {
                total: 6,
                items: [
                  { kind: 'created', path: 'app/Http/Controllers/ExportController.php', old_path: null, conflict: false },
                  { kind: 'created', path: 'resources/js/Pages/Export.vue', old_path: null, conflict: false },
                  { kind: 'modified', path: 'app/Services/InvoiceService.php', old_path: null, conflict: false },
                  { kind: 'modified', path: 'tests/Feature/InvoiceTest.php', old_path: null, conflict: true },
                  { kind: 'deleted', path: 'app/Helpers/LegacyTotals.php', old_path: null, conflict: false },
                  {
                    kind: 'renamed',
                    path: 'resources/js/Components/GuestForm.vue',
                    old_path: 'resources/js/Components/GuestEdit.vue',
                    conflict: false,
                  },
                ],
              },
            },
          }),
        );
        break;
      case 'list_past_sessions': {
        const now = Date.now();
        later(() =>
          t.emit({
            type: 'past_sessions',
            project_id: req.project_id,
            sessions: [
              { id: 'c1', last_prompt: 'why does the guest checkout page return a 419 after login?', modified_ms: now - 2 * 3_600_000 },
              { id: 'c2', last_prompt: 'refactor the booking calendar into a composable', modified_ms: now - 26 * 3_600_000 },
              { id: 'c3', last_prompt: null, modified_ms: now - 9 * 86_400_000 },
            ],
          }),
        );
        break;
      }
      case 'set_shell_panel': {
        const p = state.projects.find((x) => x.id === req.project_id);
        if (p) p.shell_panel = req.panel;
        later(emitState);
        break;
      }
      case 'wake': {
        const s = state.sessions.find((x) => x.deck_id === req.deck_id);
        if (s) {
          s.state = 'idle';
          s.error = null;
          later(() => t.emit({ type: 'session_changed', session: { ...s } }));
        }
        break;
      }
      default:
        break;
    }
  }

  t.runScene = async (deck, scene) => {
    await loadedP;
    await new Promise((r) => setTimeout(r, 50));
    const emit = (e: Event) => t.emit(e);
    switch (scene) {
      case 'main':
        deck.selectProject(P.codehive);
        deck.openSession(S.fix);
        break;
      case 'needs':
        deck.selectProject(P.codehive);
        deck.openSession(S.qa);
        break;
      case 'changes':
        deck.selectProject(P.codehive);
        deck.openSession(S.fix);
        deck.showChanges(P.codehive);
        break;
      case 'shellmin':
        deck.selectProject(P.codehive);
        deck.openSession(S.fix);
        deck.setShellPanel(P.codehive, { open: false, width: 480 });
        break;
      case 'close':
        deck.selectProject(P.codehive);
        deck.openSession(S.fix);
        deck.requestClose();
        break;
      case 'confirm':
        deck.selectProject(P.codehive);
        deck.openSession(S.fix);
        emit({
          type: 'needs_confirm',
          action: { type: 'sleep', deck_id: S.fix },
          message: 'Claude is still working in "fix invoice discount applied twice". Sleep it anyway?',
        });
        break;
      case 'split': {
        const p = state.projects.find((x) => x.id === P.codehive);
        if (p) p.layout = splitOf('row', paneOf('pane-l', S.fix, S.qa), paneOf('pane-r', S.export));
        emitState();
        await new Promise((r) => setTimeout(r, 50));
        deck.selectProject(P.codehive);
        break;
      }
      case 'groups': {
        const dio = state.sessions.find((x) => x.deck_id === S.dio);
        if (dio) dio.state = 'asleep';
        state.groups = [
          {
            id: G_BILLING,
            name: 'Billing',
            layout: splitOf(
              'column',
              splitOf('row', paneOf('g-fix', S.fix), paneOf('g-qa', S.qa)),
              splitOf('row', paneOf('g-nw', S.nwIdle), paneOf('g-dio', S.dio)),
            ),
          },
        ];
        emitState();
        await new Promise((r) => setTimeout(r, 50));
        deck.selectGroup(G_BILLING);
        deck.focusPane('g-qa');
        break;
      }
      case 'empty':
        state = { ...state, projects: [], sessions: [], shells: [], missing_folders: [] };
        emitState();
        break;
      default:
        // 'settings' and 'past' dialogs are not reachable from the store: the screenshot script clicks.
        break;
    }
  };

  return t;
}
