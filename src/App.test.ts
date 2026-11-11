import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as events from './test/eventMock';
import * as tauri from './test/tauriMock';
import type { ProviderProfile } from './lib/api/bindings/ProviderProfile';

vi.mock('@tauri-apps/api/core', () => import('./test/tauriMock'));
vi.mock('@tauri-apps/api/event', () => import('./test/eventMock'));

const { library: libraryStore } = await import('./lib/stores/library.svelte');
const { search: searchStore } = await import('./lib/stores/search.svelte');
const { details: detailsStore } = await import('./lib/stores/details.svelte');
const { providers: providersStore } = await import('./lib/stores/providers.svelte');
const { roles: rolesStore } = await import('./lib/stores/roles.svelte');
const { answer: answerStore } = await import('./lib/stores/answer.svelte');
const { trails: trailsStore } = await import('./lib/stores/trails.svelte');

const { default: App } = await import('./App.svelte');
const { view } = await import('./lib/stores/view.svelte');
const { theme } = await import('./lib/stores/theme.svelte');
const { workspace } = await import('./lib/stores/workspace.svelte');
const { sources } = await import('./lib/stores/sources.svelte');
const { inventory } = await import('./lib/stores/inventory.svelte');
const { extraction } = await import('./lib/stores/extraction.svelte');

const h1 = (name: string | RegExp) => screen.findByRole('heading', { level: 1, name });

/** Fresh install: Welcome, accept the suggested location, skip folders. */
async function onboard() {
  const user = userEvent.setup();
  render(App);
  await user.click(await screen.findByRole('button', { name: 'Get Started' }));
  await user.click(await screen.findByRole('button', { name: 'Use this location' }));
  await h1('Add the folders where you keep your books and papers');
  await user.click(screen.getByRole('button', { name: 'Skip for now' }));
  await h1('Checking this computer');
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await h1('What are you curious about?');
  return user;
}

function resetAll() {
  tauri.resetHandlers();
  view.go('loading');
  workspace.status = null;
  sources.clear();
  inventory.clear();
  extraction.clear();
  searchStore.reset();
  answerStore.reset();
  detailsStore.reset();
  trailsStore.reset();
  theme.apply('system');
}

/** Relaunch with a library already open. */
async function launchOpen() {
  tauri.setHandler('get_data_root_status', () => tauri.openStatus());
  const user = userEvent.setup();
  render(App);
  await h1('What are you curious about?');
  return user;
}

describe('launch routing', () => {
  beforeEach(resetAll);

  it('first run opens on Welcome with one action and no sign-in or AI gate', async () => {
    render(App);
    expect(await h1("Explore everything you've collected.")).toHaveFocus();
    expect(screen.getAllByRole('button')).toHaveLength(1);
    expect(screen.queryByText(/sign in/i)).not.toBeInTheDocument();
  });

  it('an open library skips Welcome and restores the saved theme', async () => {
    tauri.setHandler('get_settings', () => ({ appearance: { theme: 'dark' } }));
    await launchOpen();
    expect(screen.queryByText("Explore everything you've collected.")).not.toBeInTheDocument();
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('a missing library offers reconnect and retry, never re-creation', async () => {
    tauri.setHandler('get_data_root_status', () => ({
      state: 'missing',
      displayPath: '/Volumes/Archive/Atlas Library',
      rootId: 'r',
      problem: tauri.apiError(
        'data_root_missing',
        'Atlas cannot find its library at that location.',
        'Reconnect the drive that holds your Atlas library, then try again.',
      ),
    }));
    const user = userEvent.setup();
    render(App);
    expect(await h1("Atlas can't find your library")).toBeInTheDocument();
    expect(screen.getByText('/Volumes/Archive/Atlas Library')).toBeInTheDocument();
    expect(screen.getByText(/reconnect the drive, then try again/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /create/i })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_data_root', {
      path: '/Volumes/Archive/Atlas Library',
    });
    await h1('What are you curious about?');
  });

  it('a library in use explains the other window', async () => {
    tauri.setHandler('get_data_root_status', () => ({
      state: 'in_use',
      displayPath: '/x/Atlas Library',
      rootId: 'r',
      problem: tauri.apiError('data_root_in_use', 'This Atlas library is already open.'),
    }));
    render(App);
    expect(await h1('Your library is open in another Atlas window')).toBeInTheDocument();
  });
});

describe('choosing the data location', () => {
  beforeEach(resetAll);

  async function toDataLocation() {
    const user = userEvent.setup();
    render(App);
    await user.click(await screen.findByRole('button', { name: 'Get Started' }));
    await h1('Choose where Atlas keeps its library');
    return user;
  }

  it('shows the suggested location, free space, and the read-only promise', async () => {
    await toDataLocation();
    expect(await screen.findByTestId('candidate-path')).toHaveTextContent(tauri.DEFAULT_ROOT);
    expect(screen.getByText('245 GB of 995 GB')).toBeInTheDocument();
    expect(screen.getByText(/Your books stay where they are/)).toBeInTheDocument();
  });

  it('creates the library at the suggested path', async () => {
    await onboard();
    expect(tauri.invoke).toHaveBeenCalledWith('choose_data_root', { path: tauri.DEFAULT_ROOT });
  });

  it('offers to open an existing library', async () => {
    tauri.setHandler('inspect_data_root', () => tauri.candidate({ kind: 'existing_library' }));
    await toDataLocation();
    expect(await screen.findByRole('button', { name: 'Open this library' })).toBeEnabled();
  });

  it('explains blocking problems and disables the primary action', async () => {
    tauri.setHandler('inspect_data_root', () =>
      tauri.candidate({ problems: ['cloud_synced'], canUse: false }),
    );
    await toDataLocation();
    expect(await screen.findByText(/synced to the cloud/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Use this location' })).toBeDisabled();
  });

  it('offers an Atlas Library subfolder for a folder with other files', async () => {
    tauri.setHandler('pick_folder', () => '/Users/test/Documents');
    tauri.setHandler('inspect_data_root', (args) =>
      args?.path === '/Users/test/Documents'
        ? tauri.candidate({
            displayPath: '/Users/test/Documents',
            kind: 'non_empty_folder',
            canUse: false,
            suggestedSubfolder: '/Users/test/Documents/Atlas Library',
          })
        : tauri.candidate({ displayPath: String(args?.path) }),
    );
    const user = await toDataLocation();
    await user.click(screen.getByRole('button', { name: 'Choose another location…' }));
    await user.click(
      await screen.findByRole('button', { name: 'Create an “Atlas Library” folder here' }),
    );
    expect(await screen.findByTestId('candidate-path')).toHaveTextContent(
      '/Users/test/Documents/Atlas Library',
    );
    expect(screen.getByRole('button', { name: 'Use this location' })).toBeEnabled();
  });

  it('cancelling the folder picker keeps the current choice', async () => {
    const user = await toDataLocation();
    await screen.findByTestId('candidate-path');
    await user.click(screen.getByRole('button', { name: 'Choose another location…' }));
    expect(screen.getByTestId('candidate-path')).toHaveTextContent(tauri.DEFAULT_ROOT);
  });

  it('shows the error and recovery when creation fails', async () => {
    tauri.setHandler('choose_data_root', () => {
      throw tauri.apiError(
        'data_root_low_space',
        'Not enough free space: 900 MiB free, at least 2048 MiB needed to start safely.',
        'Free up space on that drive, or choose a drive with more room.',
      );
    });
    const user = await toDataLocation();
    await user.click(await screen.findByRole('button', { name: 'Use this location' }));
    expect(await screen.findByText(/Not enough free space/)).toBeInTheDocument();
    expect(screen.getByText(/Free up space on that drive/)).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(
      'Choose where Atlas keeps its library',
    );
  });
});

describe('main shell', () => {
  beforeEach(resetAll);

  it('shows navigation and status after onboarding', async () => {
    await onboard();
    expect(screen.getByRole('navigation', { name: 'Main' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Home' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('inventory-status')).toHaveTextContent('No library folders yet');
  });

  it('keyboard shortcuts switch views', async () => {
    const user = await launchOpen();
    await user.keyboard('{Control>}5{/Control}');
    expect(await h1('Settings')).toHaveFocus();
    await user.keyboard('{Control>}4{/Control}');
    expect(await h1('Trails')).toHaveFocus();
    await user.keyboard('{Control>}3{/Control}');
    expect(await h1('Library')).toHaveFocus();
    await user.keyboard('{Control>}2{/Control}');
    expect(await h1('Search')).toHaveFocus();
  });

  it('theme choice is applied and saved to the library settings', async () => {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('radio', { name: 'Dark' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: {
          appearance: { theme: 'dark' },
          privacy: {
            allowRemoteEmbedding: false,
            disableExternalAi: false,
            acknowledged: [],
            recordTrails: true,
            expireTrails: true,
          },
        },
      }),
    );
    await user.click(screen.getByRole('radio', { name: 'Match system' }));
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it('Settings shows where the library lives and offers a different location', async () => {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByText(tauri.DEFAULT_ROOT)).toBeInTheDocument();
    expect(await screen.findByText('245 GB of 995 GB')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Use a different location…' }));
    await h1('Choose where Atlas keeps its library');
    expect(screen.getByText(/does not move your current one/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Back' }));
    await h1('Settings');
  });
});

describe('library folders', () => {
  beforeEach(resetAll);

  async function toFoldersStep() {
    const user = userEvent.setup();
    render(App);
    await user.click(await screen.findByRole('button', { name: 'Get Started' }));
    await user.click(await screen.findByRole('button', { name: 'Use this location' }));
    await h1('Add the folders where you keep your books and papers');
    return user;
  }

  it('onboarding asks for folders with the read-only promise; Continue needs one', async () => {
    await toFoldersStep();
    expect(screen.getByText(/never moves, renames, or changes your files/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
  });

  it('adds picked folders and explains each refusal', async () => {
    tauri.pickerQueue.push({
      added: [tauri.sourceRoot()],
      rejected: [
        {
          displayPath: '/Users/test/Books/Astronomy',
          error: tauri.apiError(
            'source_overlaps',
            'That folder overlaps a folder already in your library.',
            'Keep the outer folder, or remove it first and add the folders inside it separately.',
          ),
          conflictingRootId: 'root-books',
        },
      ],
    });
    const user = await toFoldersStep();
    await user.click(screen.getByRole('button', { name: 'Add folders…' }));
    const list = await screen.findByRole('list', { name: 'Library folders' });
    expect(within(list).getByText('Books')).toBeInTheDocument();
    expect(within(list).getByText('Available')).toBeInTheDocument();
    expect(screen.getByText('Not added: Astronomy')).toBeInTheDocument();
    expect(
      screen.getByText(/overlaps a folder already in your library\. \(“Books”\)/),
    ).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Continue' }));
    await h1('Checking this computer');
    await user.click(screen.getByRole('button', { name: 'Continue' }));
    await h1('What are you curious about?');
    expect(screen.getByText('/Users/test/Books')).toBeInTheDocument();
  });

  it('cancelling the picker changes nothing', async () => {
    const user = await toFoldersStep();
    await user.click(screen.getByRole('button', { name: 'Add folders…' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pick_and_add_source_roots', undefined);
    expect(screen.queryByRole('list', { name: 'Library folders' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
  });

  it('shows status for unplugged, permission-blocked, nested, and data-root folders', async () => {
    tauri.setHandler('list_source_roots', () => [
      tauri.sourceRoot({ id: 'outer', displayPath: '/Volumes/Archive/Library' }),
      tauri.sourceRoot({ id: 'gone', displayPath: '/Volumes/Old/Papers', status: 'unavailable' }),
      tauri.sourceRoot({
        id: 'tcc',
        displayPath: '/Users/test/Documents/Scans',
        status: 'no_access',
      }),
      tauri.sourceRoot({
        id: 'inner',
        displayPath: '/Volumes/Archive/Library/Math',
        nestedInRootId: 'outer',
      }),
      tauri.sourceRoot({ id: 'home', displayPath: '/Users/test', containsDataRoot: true }),
    ]);
    await launchOpen();
    expect(screen.getByText('Not connected')).toBeInTheDocument();
    expect(screen.getByText('Atlas needs permission')).toBeInTheDocument();
    expect(screen.getByText('Inside “Library”')).toBeInTheDocument();
    expect(screen.getByText(/own data folder inside it will be skipped/)).toBeInTheDocument();
  });

  it('removing a folder needs confirmation and keeps files', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await h1('Settings');
    await user.click(await screen.findByRole('button', { name: 'Remove Books from Atlas' }));
    expect(
      screen.getByText('Remove from Atlas? Your files stay where they are.'),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Keep' }));
    expect(tauri.invoke).not.toHaveBeenCalledWith('remove_source_root', expect.anything());

    await user.click(screen.getByRole('button', { name: 'Remove Books from Atlas' }));
    tauri.setHandler('list_source_roots', () => []);
    await user.click(screen.getByRole('button', { name: 'Remove' }));
    expect(tauri.invoke).toHaveBeenCalledWith('remove_source_root', { id: 'root-books' });
    expect(
      await screen.findAllByText('Choose a folder to start discovering your library.'),
    ).not.toHaveLength(0);
  });

  it('switching to an existing library with folders skips the folders step', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('inspect_data_root', () => tauri.candidate({ kind: 'existing_library' }));
    const user = userEvent.setup();
    render(App);
    await user.click(await screen.findByRole('button', { name: 'Get Started' }));
    await user.click(await screen.findByRole('button', { name: 'Open this library' }));
    await h1('What are you curious about?');
  });
});

describe('device check', () => {
  beforeEach(resetAll);

  async function toDeviceCheck() {
    const user = userEvent.setup();
    render(App);
    await user.click(await screen.findByRole('button', { name: 'Get Started' }));
    await user.click(await screen.findByRole('button', { name: 'Use this location' }));
    await user.click(await screen.findByRole('button', { name: 'Skip for now' }));
    await h1('Checking this computer');
    return user;
  }

  it('shows readings, a plain recommendation, notes, and the provisional caveat', async () => {
    await toDeviceCheck();
    expect(await screen.findByText('64 GB')).toBeInTheDocument();
    expect(screen.getByText('18 cores')).toBeInTheDocument();
    expect(screen.getByText('712 GB free of 2.0 TB')).toBeInTheDocument();
    expect(screen.getByText('On battery · 75%')).toBeInTheDocument();
    expect(screen.getByText(/can likely run AI models locally/)).toBeInTheDocument();
    expect(screen.getByText(/catalog slowly to save power/)).toBeInTheDocument();
    expect(screen.getByText(/early estimates, not tested requirements/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Continue' })).toBeEnabled();
  });

  it.each([
    [{ power: 'ac', charging: true, batteryPercent: 80 }, 'Charging · 80%'],
    [{ power: 'ac', charging: false, batteryPercent: 100 }, 'Plugged in · 100%'],
    [
      { power: 'ac', batteryPresent: false, batteryPercent: null, charging: null },
      'No battery (desktop)',
    ],
    [{ power: 'unknown', batteryPresent: null, batteryPercent: null, charging: null }, 'Unknown'],
  ] as const)('renders power %#', async (overrides, text) => {
    tauri.setHandler('get_device_check', () =>
      tauri.deviceCheck({ snapshot: tauri.deviceSnapshot(overrides) }),
    );
    await toDeviceCheck();
    expect(await screen.findByText(text)).toBeInTheDocument();
  });

  it('shows Unknown for missing readings instead of guessing', async () => {
    tauri.setHandler('get_device_check', () =>
      tauri.deviceCheck({
        snapshot: tauri.deviceSnapshot({ totalMemoryBytes: null, logicalCores: null }),
        assessment: { library: 'unknown', localAi: 'unknown', notes: ['unknown_memory'] },
      }),
    );
    await toDeviceCheck();
    expect(await screen.findAllByText('Unknown')).toHaveLength(2);
    expect(
      screen.getByText(/couldn't read this computer's memory, so it will start conservatively/),
    ).toBeInTheDocument();
  });

  it('a modest computer gets an honest recommendation', async () => {
    tauri.setHandler('get_device_check', () =>
      tauri.deviceCheck({ assessment: { library: 'comfortable', localAi: 'possible', notes: [] } }),
    );
    await toDeviceCheck();
    expect(
      await screen.findByText(
        'This computer can comfortably catalog and search your books. Local AI may be slower; you can connect a service instead.',
      ),
    ).toBeInTheDocument();
  });

  it.each([
    [
      { mode: 'run', pauseReason: null, throttleReason: null, maxMetadataOpsPerSecond: 100 },
      {},
      /it will run normally\./,
    ],
    [
      {
        mode: 'slow',
        pauseReason: null,
        throttleReason: 'on_battery',
        maxMetadataOpsPerSecond: 20,
      },
      {},
      /run slowly because this computer is on battery/,
    ],
    [
      {
        mode: 'paused',
        pauseReason: 'battery_low',
        throttleReason: null,
        maxMetadataOpsPerSecond: null,
      },
      { batteryPercent: 18 },
      /battery is at 18%\. It resumes at 25% or when you plug in/,
    ],
    [
      {
        mode: 'paused',
        pauseReason: 'low_disk_space',
        throttleReason: null,
        maxMetadataOpsPerSecond: null,
      },
      {},
      /nearly full\. It resumes when at least 10\.7 GB is free/,
    ],
  ] as const)('Settings explains the policy %#', async (policy, snapshot, sentence) => {
    tauri.setHandler('get_device_check', () =>
      tauri.deviceCheck({ policy, snapshot: tauri.deviceSnapshot(snapshot) }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const line = await screen.findByTestId('policy-sentence');
    expect(line).toHaveTextContent(sentence);
    // Cataloging exists; the copy must not promise it for a later slice.
    expect(line).not.toHaveTextContent(/slice|coming/i);
  });
});

describe('cataloging status', () => {
  beforeEach(resetAll);

  async function launchWith(ov: ReturnType<typeof tauri.overview>) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_inventory_status', () => ov);
    return launchOpen();
  }

  const status = () => screen.getByTestId('inventory-status');

  it.each([
    [{ state: 'running' }, 'Cataloged 1,204 files · Scanning Books'],
    [
      { state: 'running', throttleReason: 'on_battery' },
      'Cataloged 1,204 files · Scanning Books · Slowed on battery',
    ],
    [{ state: 'queued' }, 'Waiting to catalog Books'],
    [{ state: 'paused_user' }, 'Paused by you · 1,204 files so far'],
    [
      { state: 'paused_policy', pauseReason: 'battery_low' },
      'Paused on battery · Resumes at 25% or when plugged in',
    ],
    [
      { state: 'paused_policy', pauseReason: 'low_disk_space' },
      "Paused · The drive with Atlas's data is nearly full",
    ],
    [
      { state: 'paused_policy', pauseReason: 'source_root_missing' },
      "Paused · “Books” isn't available",
    ],
  ] as const)('status line %#', async (active, text) => {
    await launchWith(tauri.overview({ active: tauri.inventoryStatus(active) }));
    expect(status()).toHaveTextContent(text);
    expect(status().textContent).not.toMatch(/%(?! or)|percent|of \d/);
  });

  it('is up to date when nothing is scheduled', async () => {
    await launchWith(
      tauri.overview({
        catalogedFiles: 1204,
        roots: [
          {
            sourceRootId: 'root-books',
            fileCount: 1204,
            missingCount: 3,
            cloudOnlyCount: 12,
            lastScanAt: new Date(Date.now() - 5 * 60_000).toISOString(),
            lastState: 'completed',
            activeJobId: null,
          },
        ],
      }),
    );
    expect(status()).toHaveTextContent('Up to date · 1,204 files');
    expect(screen.getByTestId('root-line')).toHaveTextContent(
      '1,204 files · Updated 5 min ago · 12 only in the cloud · 3 missing',
    );
  });

  it('Pause, Resume, and Cancel call the right commands', async () => {
    const user = await launchWith(tauri.overview({ active: tauri.inventoryStatus() }));
    await user.click(screen.getByRole('button', { name: 'Pause' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pause_job', { jobId: 'job-1' });

    events.emit(
      'inventory-status',
      tauri.overview({ active: tauri.inventoryStatus({ state: 'paused_user' }) }),
    );
    await user.click(await screen.findByRole('button', { name: 'Resume' }));
    expect(tauri.invoke).toHaveBeenCalledWith('resume_job', { jobId: 'job-1' });

    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(tauri.invoke).toHaveBeenCalledWith('cancel_job', { jobId: 'job-1' });
  });

  it('live events update the status bar', async () => {
    await launchWith(tauri.overview({ active: tauri.inventoryStatus({ discoveredFiles: 10 }) }));
    expect(status()).toHaveTextContent('Cataloged 10 files');
    events.emit(
      'inventory-status',
      tauri.overview({ active: tauri.inventoryStatus({ discoveredFiles: 2500 }) }),
    );
    await waitFor(() => expect(status()).toHaveTextContent('Cataloged 2,500 files'));
  });

  it('Rescan starts cataloging a folder', async () => {
    const user = await launchWith(tauri.overview());
    await user.click(await screen.findByRole('button', { name: 'Rescan Books' }));
    expect(tauri.invoke).toHaveBeenCalledWith('start_inventory', { sourceRootId: 'root-books' });
  });

  it('details show counts and time without any total', async () => {
    const user = await launchWith(tauri.overview({ active: tauri.inventoryStatus() }));
    await user.click(screen.getByText('Details'));
    expect(screen.getByText('37')).toBeInTheDocument();
    expect(screen.getByText('1 min 23 s')).toBeInTheDocument();
  });
});

describe('library table', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const books = [
    tauri.fileInstance({ id: 'f1', displayName: 'Café crème.pdf', sizeBytes: 48_200_000 }),
    tauri.fileInstance({
      id: 'f2',
      displayName: 'Cosmos.pdf',
      storage: 'cloud_only',
      sizeBytes: 48_200_000,
    }),
    tauri.fileInstance({
      id: 'f3',
      displayName: 'Gone.epub',
      fileType: 'epub',
      availability: 'missing',
    }),
    tauri.fileInstance({ id: 'f4', displayName: 'Away.pdf', availability: 'unreachable' }),
  ];

  async function openLibrary(page = tauri.documentPage({ items: books, matchedSoFar: 4 })) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_inventory_status', () => tauri.overview({ catalogedFiles: 4 }));
    tauri.setHandler('list_documents', () => page);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await screen.findByRole('table');
    return user;
  }

  const lastQuery = () => {
    const calls = tauri.invoke.mock.calls.filter(([c]) => c === 'list_documents');
    return (calls.at(-1)![1] as { query: Record<string, unknown> }).query;
  };

  it('lists files with honest status badges and no invented totals', async () => {
    await openLibrary();
    const rows = within(screen.getByRole('table')).getAllByRole('row');
    expect(rows).toHaveLength(5);
    // In the table: the Text status filter also offers "Only in the cloud".
    expect(within(screen.getByRole('table')).getByText('Only in the cloud')).toBeInTheDocument();
    expect(screen.getByText('Missing')).toBeInTheDocument();
    expect(screen.getByText('Not connected')).toBeInTheDocument();
    expect(screen.getByTestId('range')).toHaveTextContent('Showing 1–4 of 4 matches');
    expect(screen.getByRole('button', { name: 'Open Gone.epub' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Open Away.pdf' })).toBeDisabled();
  });

  it('says "so far" while cataloging continues', async () => {
    await openLibrary(tauri.documentPage({ items: books, matchedSoFar: 4, scanning: true }));
    expect(screen.getByTestId('range')).toHaveTextContent('of 4 matches so far');
  });

  it('search is debounced and sends the text', async () => {
    const user = await openLibrary();
    tauri.invoke.mockClear();
    await user.type(screen.getByPlaceholderText('Search file names and folders'), 'cafe');
    await waitFor(() => expect(lastQuery().text).toBe('cafe'));
    const searches = tauri.invoke.mock.calls.filter(([c]) => c === 'list_documents');
    expect(searches.length).toBeLessThan(4);
  });

  it('filters and sorting send the right query', async () => {
    const user = await openLibrary();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Type' }), 'epub');
    expect(lastQuery()).toMatchObject({ fileType: 'epub', page: 0 });
    await user.click(screen.getByRole('checkbox', { name: 'Include missing files' }));
    expect(lastQuery()).toMatchObject({ includeMissing: true });
    await user.selectOptions(screen.getByRole('combobox', { name: 'Folder' }), 'root-books');
    expect(lastQuery()).toMatchObject({ sourceRootId: 'root-books' });

    const nameHeader = screen.getByRole('columnheader', { name: 'Name' });
    expect(nameHeader).toHaveAttribute('aria-sort', 'ascending');
    await user.click(within(nameHeader).getByRole('button'));
    expect(lastQuery()).toMatchObject({ sort: 'name_desc' });
    await user.click(
      within(screen.getByRole('columnheader', { name: 'Modified' })).getByRole('button'),
    );
    expect(lastQuery()).toMatchObject({ sort: 'modified_desc' });
  });

  it('pages through results', async () => {
    const items = Array.from({ length: 50 }, (_, i) =>
      tauri.fileInstance({ id: `p${i}`, displayName: `paper ${i}.pdf` }),
    );
    const user = await openLibrary(tauri.documentPage({ items, matchedSoFar: 120 }));
    expect(screen.getByTestId('range')).toHaveTextContent('Showing 1–50 of 120 matches');
    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(lastQuery()).toMatchObject({ page: 1 });
  });

  it('opens and reveals by file ID', async () => {
    const user = await openLibrary();
    await user.click(screen.getByRole('button', { name: 'Open Café crème.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_original', {
      fileId: 'f1',
      allowDownload: false,
    });
    await user.click(screen.getByRole('button', { name: 'Show in Finder: Café crème.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('reveal_original', { fileId: 'f1' });
  });

  it('asks before downloading a cloud-only file', async () => {
    tauri.setHandler('open_original', (args) => {
      if (!args?.allowDownload) {
        throw tauri.apiError('file_cloud_only', 'This file is stored only in the cloud.');
      }
      return null;
    });
    const user = await openLibrary();
    await user.click(screen.getByRole('button', { name: 'Open Cosmos.pdf' }));
    expect(
      await screen.findByText(
        'This file is stored in the cloud. Opening it will download it (48.2 MB).',
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Download and open' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_original', {
      fileId: 'f2',
      allowDownload: true,
    });
    await waitFor(() =>
      expect(screen.queryByText(/Opening it will download it/)).not.toBeInTheDocument(),
    );
  });

  it('explains a missing file with a recovery step', async () => {
    tauri.setHandler('open_original', () => {
      throw tauri.apiError(
        'file_missing',
        'That file is no longer where Atlas found it.',
        'It may have been moved, renamed, or deleted. Rescan its folder to update the library.',
      );
    });
    const user = await openLibrary();
    await user.click(screen.getByRole('button', { name: 'Open Café crème.pdf' }));
    expect(await screen.findByText("Couldn't open Café crème.pdf")).toBeInTheDocument();
    expect(screen.getByText(/Rescan its folder/)).toBeInTheDocument();
  });

  it('shows a helpful empty state for a search with no matches', async () => {
    const user = await openLibrary();
    tauri.setHandler('list_documents', () => tauri.documentPage());
    await user.type(screen.getByPlaceholderText('Search file names and folders'), 'zebra');
    expect(await screen.findByText('No files match “zebra”.')).toBeInTheDocument();
  });

  it('refreshes the page as cataloging progresses, at most once a second', async () => {
    await openLibrary(tauri.documentPage({ items: books, matchedSoFar: 4, scanning: true }));
    tauri.invoke.mockClear();
    for (let i = 0; i < 5; i++) {
      events.emit('inventory-status', tauri.overview({ catalogedFiles: 10 + i }));
    }
    await waitFor(
      () => expect(tauri.invoke.mock.calls.filter(([c]) => c === 'list_documents').length).toBe(1),
      { timeout: 2000 },
    );
  });
});

describe('reading text', () => {
  beforeEach(resetAll);

  async function launchWith(ov: ReturnType<typeof tauri.extractionOverview>) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_extraction_status', () => ov);
    return launchOpen();
  }

  const line = () => screen.getByTestId('extraction-status');
  const counts = tauri.extractionCounts({ done: 3, pending: 7, total: 10 });

  it.each([
    [{ state: 'running' }, false, 'Reading text · 3 of 10 files'],
    [{ state: 'running' }, true, 'Reading text · 3 files so far'],
    [
      { state: 'running', throttleReason: 'on_battery' },
      false,
      'Reading text · 3 of 10 files · Slowed on battery',
    ],
    [
      { state: 'paused_policy', pauseReason: 'battery_low' },
      false,
      'Paused reading text · Resumes at 25% or when plugged in',
    ],
    [{ state: 'paused_user' }, false, 'Paused reading text by you · 3 of 10 files'],
    [{ state: 'queued' }, false, 'Waiting to read text · 3 of 10 files'],
  ] as const)('status line %#', async (active, scanning, text) => {
    await launchWith(
      tauri.extractionOverview({ active: tauri.extractionStatus(active), counts, scanning }),
    );
    expect(line()).toHaveTextContent(text);
    if (scanning) expect(line().textContent).not.toMatch(/of \d/);
  });

  it('says when text is being read again by a newer reader', async () => {
    const updating = tauri.extractionCounts({ done: 10, total: 10, updating: 4 });
    await launchWith(
      tauri.extractionOverview({ active: tauri.extractionStatus(), counts: updating }),
    );
    expect(line()).toHaveTextContent('Updating text · 4 files to go');
    events.emit('extraction-status', tauri.extractionOverview({ counts: updating }));
    await waitFor(() =>
      expect(line()).toHaveTextContent('Text waiting to be updated · 4 files to go'),
    );
  });

  it('is up to date when every file has a status', async () => {
    await launchWith(
      tauri.extractionOverview({
        counts: tauri.extractionCounts({ done: 8, needsOcr: 1, encrypted: 1, total: 10 }),
      }),
    );
    expect(line()).toHaveTextContent('Text up to date · 8 of 10 files readable');
    expect(screen.queryByRole('button', { name: 'Pause reading text' })).toBeNull();
  });

  it('shows nothing before any file is cataloged', async () => {
    await launchWith(tauri.extractionOverview());
    expect(screen.queryByTestId('extraction-status')).toBeNull();
  });

  it('pauses, resumes and cancels the reading job, not the scan', async () => {
    const user = await launchWith(
      tauri.extractionOverview({ active: tauri.extractionStatus(), counts }),
    );
    await user.click(screen.getByRole('button', { name: 'Pause reading text' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pause_job', { jobId: 'extract-1' });
    events.emit(
      'extraction-status',
      tauri.extractionOverview({
        active: tauri.extractionStatus({ state: 'paused_user' }),
        counts,
      }),
    );
    await user.click(await screen.findByRole('button', { name: 'Resume reading text' }));
    expect(tauri.invoke).toHaveBeenCalledWith('resume_job', { jobId: 'extract-1' });
    await user.click(screen.getByRole('button', { name: 'Cancel reading text' }));
    expect(tauri.invoke).toHaveBeenCalledWith('cancel_job', { jobId: 'extract-1' });
  });

  it("says when a folder's reading is paused", async () => {
    await launchWith(
      tauri.extractionOverview({
        active: tauri.extractionStatus({ state: 'paused_user' }),
        counts,
        roots: [
          {
            sourceRootId: 'root-books',
            counts,
            lastState: 'paused_user',
            activeJobId: 'extract-1',
          },
        ],
      }),
    );
    expect(await screen.findByTestId('root-text-line')).toHaveTextContent('Reading text paused');
  });

  it('reads a folder now from its line', async () => {
    const user = await launchWith(tauri.extractionOverview());
    await user.click(await screen.findByRole('button', { name: 'Read text now in Books' }));
    expect(tauri.invoke).toHaveBeenCalledWith('start_extraction', { sourceRootId: 'root-books' });
  });

  const files = [
    tauri.fileInstance({
      id: 'r1',
      displayName: 'Ready.pdf',
      extraction: { state: 'done', reason: null },
    }),
    tauri.fileInstance({ id: 'r2', displayName: 'Now.pdf' }),
    tauri.fileInstance({
      id: 'r3',
      displayName: 'Scan.pdf',
      extraction: { state: 'needs_ocr', reason: 'no_text_layer' },
    }),
    tauri.fileInstance({
      id: 'r4',
      displayName: 'Locked.epub',
      fileType: 'epub',
      extraction: { state: 'encrypted', reason: 'drm' },
    }),
    tauri.fileInstance({
      id: 'r5',
      displayName: 'Away.pdf',
      storage: 'cloud_only',
      extraction: { state: 'cloud_only', reason: null },
    }),
    tauri.fileInstance({
      id: 'r6',
      displayName: 'Broken.pdf',
      extraction: { state: 'unreadable', reason: 'not_a_pdf' },
    }),
    tauri.fileInstance({ id: 'r7', displayName: 'Later.pdf' }),
  ];

  async function openLibrary() {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('list_documents', () =>
      tauri.documentPage({ items: files, matchedSoFar: files.length }),
    );
    tauri.setHandler('get_extraction_status', () =>
      tauri.extractionOverview({
        active: tauri.extractionStatus({ currentFileId: 'r2' }),
        counts: tauri.extractionCounts({ done: 1, pending: 2, total: 7 }),
      }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await screen.findByRole('table');
    return user;
  }

  it('shows a Text badge with a symbol and the reason for every file', async () => {
    await openLibrary();
    const badges = screen.getAllByTestId('text-status').map((b) => b.textContent?.trim());
    expect(badges).toEqual([
      '✓ Text ready',
      '↻ Reading…',
      '⊘ Needs OCR',
      '🔒 Encrypted',
      '☁ Only in the cloud',
      "! Couldn't read",
      '… Waiting',
    ]);
    const scan = screen.getAllByTestId('text-status')[2];
    expect(scan).toHaveAttribute('title', expect.stringMatching(/looks scanned/));
    expect(screen.getAllByTestId('text-status')[0]).not.toHaveAttribute('title');
  });

  it('refreshes the page as text is read, while cataloging status is also present', async () => {
    await openLibrary();
    tauri.invoke.mockClear();
    events.emit(
      'extraction-status',
      tauri.extractionOverview({ counts: tauri.extractionCounts({ done: 7, total: 7 }) }),
    );
    await waitFor(
      () => expect(tauri.invoke.mock.calls.filter(([c]) => c === 'list_documents').length).toBe(1),
      { timeout: 2000 },
    );
  });

  it('filters by text status', async () => {
    const user = await openLibrary();
    const calls = () => tauri.invoke.mock.calls.filter(([c]) => c === 'list_documents');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Text status' }), 'needs_ocr');
    await waitFor(() =>
      expect((calls().at(-1)![1] as { query: Record<string, unknown> }).query.textStatus).toBe(
        'needs_ocr',
      ),
    );
  });
});

describe('AI models settings', () => {
  beforeEach(() => {
    resetAll();
    providersStore.clear();
    rolesStore.clear();
  });

  async function openAiSettings() {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { level: 2, name: 'AI Models' });
    return user;
  }

  const aiSection = () =>
    screen.getByRole('heading', { level: 2, name: 'AI Models' }).closest('section') as HTMLElement;

  it('explains it is optional and that nothing from the library is sent', async () => {
    await openAiSettings();
    expect(within(aiSection()).getByText(/nothing from your library is sent/)).toBeInTheDocument();
    expect(within(aiSection()).getByText('No AI services connected.')).toBeInTheDocument();
  });

  it('adds Ollama from a preset, tests before saving, and saves', async () => {
    tauri.setHandler('test_provider_connection', () =>
      tauri.connectionResult({
        outcome: 'no_models_listed',
        modelIds: [],
        serverVersion: '0.40.15',
      }),
    );
    const user = await openAiSettings();
    await user.click(within(aiSection()).getByRole('button', { name: 'Add a service…' }));
    const form = within(aiSection()).getByRole('form', { name: 'Add an AI service' });
    expect(within(form).getByLabelText('Address')).toHaveValue('http://localhost:11434');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(
      await within(aiSection()).findByText(
        'Ollama 0.40.15 is running but has no models installed. Install one with Ollama, then test again.',
      ),
    ).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith('test_provider_connection', {
      target: expect.objectContaining({ target: 'candidate' }),
    });
    await user.click(within(form).getByRole('button', { name: 'Save' }));
    const services = await within(aiSection()).findByRole('list', { name: 'AI services' });
    expect(within(services).getByText('Ollama')).toBeInTheDocument();
    expect(within(services).getByText(/This computer/)).toBeInTheDocument();
  });

  it('stores a key without ever showing it again, and explains key problems', async () => {
    const user = await openAiSettings();
    await user.click(within(aiSection()).getByRole('button', { name: 'Add a service…' }));
    const form = within(aiSection()).getByRole('form', { name: 'Add an AI service' });
    await user.selectOptions(within(form).getByRole('combobox', { name: 'Service' }), 'omlx');
    expect(within(form).getByLabelText('Address')).toHaveValue('http://localhost:8000/v1');

    tauri.setHandler('test_provider_connection', () =>
      tauri.connectionResult({
        outcome: 'authentication_failed',
        modelIds: [],
        message: 'The service replied with HTTP 401.',
      }),
    );
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(
      await within(aiSection()).findByText(
        "oMLX needs an API key. Enter the key from oMLX's settings.",
      ),
    ).toBeInTheDocument();

    await user.type(within(form).getByLabelText(/API key/), 'sk-secret-123');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(
      await within(aiSection()).findByText('The key was rejected. Check it and try again.'),
    ).toBeInTheDocument();

    await user.click(within(form).getByRole('button', { name: 'Save' }));
    expect(await within(aiSection()).findByText('Key saved in Keychain')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain('sk-secret-123');
    expect(tauri.invoke).toHaveBeenCalledWith('save_provider_profile', {
      profile: expect.objectContaining({ apiKey: 'sk-secret-123', kind: 'openai_compatible' }),
    });
  });

  it('lists models without claiming what they can do', async () => {
    const user = await openAiSettings();
    await user.click(within(aiSection()).getByRole('button', { name: 'Add a service…' }));
    const form = within(aiSection()).getByRole('form', { name: 'Add an AI service' });
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(await within(aiSection()).findByText('qwen3.5:9b')).toBeInTheDocument();
    expect(
      within(aiSection()).getByText(/Listing a model doesn't show what it can do/),
    ).toBeInTheDocument();
    // Only a passing capability test makes a claim (see "Model roles").
    expect(aiSection().textContent).not.toMatch(/works for (chat|embeddings)/i);
    expect(tauri.invoke).not.toHaveBeenCalledWith('test_model_capability', expect.anything());
  });

  it('requires an acknowledgement for plain HTTP to another computer', async () => {
    const user = await openAiSettings();
    await user.click(within(aiSection()).getByRole('button', { name: 'Add a service…' }));
    const form = within(aiSection()).getByRole('form', { name: 'Add an AI service' });
    const address = within(form).getByLabelText('Address');
    await user.clear(address);
    await user.type(address, 'http://192.168.1.20:11434');
    expect(within(form).getByText(/unencrypted HTTP to another computer/)).toBeInTheDocument();
    await user.click(within(form).getByRole('checkbox', { name: /I trust this network/ }));
    await user.click(within(form).getByRole('button', { name: 'Save' }));
    expect(tauri.invoke).toHaveBeenCalledWith('save_provider_profile', {
      profile: expect.objectContaining({ insecureHttpAcknowledged: true }),
    });
    expect(await within(aiSection()).findByText(/Another computer/)).toBeInTheDocument();
  });

  it('looks for local services only when asked and prefills the form', async () => {
    tauri.setHandler('detect_local_providers', () => [
      {
        kind: 'ollama',
        label: 'Ollama',
        baseUrl: 'http://localhost:11434',
        needsKey: false,
        modelCount: null,
      },
      {
        kind: 'openai_compatible',
        label: 'oMLX',
        baseUrl: 'http://localhost:8000/v1',
        needsKey: true,
        modelCount: null,
      },
    ]);
    const user = await openAiSettings();
    expect(tauri.invoke).not.toHaveBeenCalledWith('detect_local_providers', undefined);
    await user.click(
      within(aiSection()).getByRole('button', { name: 'Look for AI services on this computer' }),
    );
    await user.click(
      await within(aiSection()).findByRole('button', { name: /Add oMLX .*needs a key/ }),
    );
    const form = within(aiSection()).getByRole('form', { name: 'Add an AI service' });
    expect(within(form).getByLabelText('Address')).toHaveValue('http://localhost:8000/v1');
    expect(within(form).getByLabelText('Name')).toHaveValue('oMLX');
  });

  it('tests and removes a saved service with confirmation', async () => {
    tauri.setHandler('list_provider_profiles', () => [
      {
        id: 'p1',
        kind: 'ollama',
        label: 'Ollama',
        baseUrl: 'http://localhost:11434',
        hasCredential: false,
        credentialStorage: 'none',
        insecureHttpAcknowledged: false,
        location: 'this_computer',
        createdAt: 't',
        updatedAt: 't',
      },
    ]);
    tauri.setHandler('test_provider_connection', () =>
      tauri.connectionResult({
        outcome: 'network_error',
        modelIds: [],
        message: 'Nothing answered at this address.',
      }),
    );
    const user = await openAiSettings();
    await user.click(await within(aiSection()).findByRole('button', { name: 'Test Ollama' }));
    expect(tauri.invoke).toHaveBeenCalledWith('test_provider_connection', {
      target: { target: 'saved_profile', profileId: 'p1' },
    });
    expect(
      await within(aiSection()).findByText(
        'Nothing answered at this address. Is the service running?',
      ),
    ).toBeInTheDocument();
    await user.click(within(aiSection()).getByRole('button', { name: 'Remove Ollama' }));
    expect(within(aiSection()).getByText('Remove this service and its key?')).toBeInTheDocument();
    await user.click(within(aiSection()).getByRole('button', { name: 'Remove' }));
    expect(tauri.invoke).toHaveBeenCalledWith('delete_provider_profile', { id: 'p1' });
  });
});

describe('Model roles', () => {
  beforeEach(() => {
    resetAll();
    providersStore.clear();
    rolesStore.clear();
  });

  const omlx: ProviderProfile = {
    id: 'p1',
    kind: 'openai_compatible',
    label: 'oMLX',
    baseUrl: 'http://localhost:8000/v1',
    hasCredential: true,
    credentialStorage: 'keychain',
    insecureHttpAcknowledged: false,
    location: 'this_computer',
    createdAt: 't',
    updatedAt: 't',
  };

  async function openModels(profiles = [omlx]) {
    tauri.setHandler('list_provider_profiles', () => profiles);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { level: 3, name: 'Models' });
    return user;
  }

  const role = (name: 'Embeddings' | 'Chat') => screen.getByRole('group', { name });

  it('explains each role, and without a service recommends what to get', async () => {
    const user = await openModels([]);
    expect(within(role('Embeddings')).getByText('Not set.')).toBeInTheDocument();
    expect(
      within(role('Embeddings')).getByText(/Add the AI service that runs it above/),
    ).toBeInTheDocument();
    const rec = await screen.findByRole('group', { name: 'Recommended embeddings model' });
    expect(within(rec).getByText('Qwen3-Embedding 0.6B')).toBeInTheDocument();
    expect(within(rec).getByText('Atlas never downloads models itself.')).toBeInTheDocument();
    // On a Mac with no service yet, oMLX's steps come first.
    expect(within(rec).getByRole('radio', { name: 'oMLX' })).toBeChecked();
    expect(within(rec).getByText(/Not yet checked with oMLX/)).toBeInTheDocument();
    await user.click(within(rec).getByRole('radio', { name: 'Ollama' }));
    expect(within(rec).getByText('ollama pull qwen3-embedding:0.6b')).toBeInTheDocument();
    expect(within(rec).queryByText(/Not yet checked/)).not.toBeInTheDocument();
    expect(screen.getByRole('group', { name: 'Recommended chat model' })).toHaveTextContent(
      'Qwen3.5 9B',
    );
  });

  it('tests a model and offers Use only after it passes', async () => {
    let answer = tauri.capabilityResult({
      role: 'embedding',
      modelId: 'Qwen3.8-27B-MLX-4bit',
      outcome: 'wrong_role',
      dimensions: null,
    });
    tauri.setHandler('test_model_capability', () => answer);
    const user = await openModels();
    await user.click(
      within(role('Embeddings')).getByRole('button', { name: 'Choose a model for embeddings' }),
    );
    const form = screen.getByRole('form', { name: 'Choose the embeddings model' });
    await user.type(within(form).getByLabelText('Model'), 'Qwen3.8-27B-MLX-4bit');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(
      await within(form).findByText("This model can't make embeddings. Choose an embedding model."),
    ).toBeInTheDocument();
    expect(within(form).queryByRole('button', { name: 'Use' })).not.toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith('test_model_capability', {
      target: { target: 'saved_profile', profileId: 'p1' },
      role: 'embedding',
      modelId: 'Qwen3.8-27B-MLX-4bit',
    });

    const model = within(form).getByLabelText('Model');
    await user.clear(model);
    await user.type(model, 'mlx-community/Qwen3-Embedding-0.6B-8bit');
    answer = tauri.capabilityResult({ modelId: 'mlx-community/Qwen3-Embedding-0.6B-8bit' });
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(
      await within(form).findByText('Works for embeddings · 1,024 dimensions · answered in 0.4 s'),
    ).toBeInTheDocument();

    // Changing the model after the test hides Use again.
    await user.type(model, 'x');
    expect(within(form).queryByRole('button', { name: 'Use' })).not.toBeInTheDocument();
    await user.type(model, '{Backspace}');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    await user.click(await within(form).findByRole('button', { name: 'Use' }));
    expect(tauri.invoke).toHaveBeenCalledWith('set_model_role', {
      role: 'embedding',
      profileId: 'p1',
      modelId: 'mlx-community/Qwen3-Embedding-0.6B-8bit',
      result: expect.objectContaining({ outcome: 'works', dimensions: 1024 }),
    });
    expect(await screen.findByTestId('role-embedding')).toHaveTextContent(
      'oMLX · mlx-community/Qwen3-Embedding-0.6B-8bit · 1,024 dimensions · tested 9 Nov',
    );
    expect(
      screen.queryByRole('group', { name: 'Recommended embeddings model' }),
    ).not.toBeInTheDocument();
  });

  it('confirms before a new embedding model replaces the meaning index', async () => {
    tauri.setHandler('get_embedding_status', () =>
      tauri.embeddingOverview({
        model: { name: 'old-embed', dims: 1024, profileLabel: 'oMLX', location: 'this_computer' },
        needsModel: false,
        hasAiService: true,
        counts: tauri.embeddingCounts({ indexed: 5, total: 5 }),
      }),
    );
    tauri.setHandler('test_model_capability', () =>
      tauri.capabilityResult({ modelId: 'new-embed', dimensions: 512 }),
    );
    const user = await openModels();
    // An embedding model in use, with an index.
    await user.click(
      within(role('Embeddings')).getByRole('button', { name: 'Choose a model for embeddings' }),
    );
    let form = screen.getByRole('form', { name: 'Choose the embeddings model' });
    tauri.setHandler('test_model_capability', () =>
      tauri.capabilityResult({ modelId: 'old-embed', dimensions: 1024 }),
    );
    await user.type(within(form).getByLabelText('Model'), 'old-embed');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    await user.click(await within(form).findByRole('button', { name: 'Use' }));
    await screen.findByTestId('role-embedding');

    await user.click(screen.getByRole('button', { name: 'Change the embeddings model' }));
    form = screen.getByRole('form', { name: 'Choose the embeddings model' });
    tauri.setHandler('test_model_capability', () =>
      tauri.capabilityResult({ modelId: 'new-embed', dimensions: 512 }),
    );
    const model = within(form).getByLabelText('Model');
    await user.clear(model);
    await user.type(model, 'new-embed');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    await user.click(await within(form).findByRole('button', { name: 'Use' }));
    const confirm = screen.getByRole('group', { name: 'Confirm a new meaning index' });
    expect(confirm).toHaveTextContent('The current one keeps working until the new one is ready.');
    expect(tauri.invoke).not.toHaveBeenCalledWith(
      'set_model_role',
      expect.objectContaining({ modelId: 'new-embed' }),
    );
    await user.click(within(confirm).getByRole('button', { name: 'Build a new index' }));
    expect(tauri.invoke).toHaveBeenCalledWith(
      'set_model_role',
      expect.objectContaining({ modelId: 'new-embed' }),
    );
  });

  it('says it may take a minute while the model loads', async () => {
    let finish: (r: unknown) => void = () => {};
    tauri.setHandler('test_model_capability', () => new Promise((resolve) => (finish = resolve)));
    const user = await openModels();
    await user.click(within(role('Chat')).getByRole('button', { name: 'Choose a model for chat' }));
    const form = screen.getByRole('form', { name: 'Choose the chat model' });
    await user.type(within(form).getByLabelText('Model'), 'Qwen3.8-27B-MLX-4bit');
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(await within(form).findByText(/Loading it can take a minute/)).toBeInTheDocument();
    expect(within(form).getByRole('button', { name: 'Test' })).toBeDisabled();
    finish(
      tauri.capabilityResult({ role: 'chat', modelId: 'Qwen3.8-27B-MLX-4bit', elapsedMs: 30 }),
    );
    expect(
      await within(form).findByText('Works for chat · answered in under 0.1 s'),
    ).toBeInTheDocument();
  });

  it('clears a role, and removing its service clears it too', async () => {
    const user = await openModels();
    for (const [name, label] of [
      ['Embeddings', 'embeddings'],
      ['Chat', 'chat'],
    ] as const) {
      await user.click(
        within(role(name)).getByRole('button', { name: `Choose a model for ${label}` }),
      );
      const form = screen.getByRole('form', { name: `Choose the ${label} model` });
      await user.type(
        within(form).getByLabelText('Model'),
        name === 'Chat' ? 'qwen3.5:9b' : 'qwen3-embedding:0.6b',
      );
      await user.click(within(form).getByRole('button', { name: 'Test' }));
      await user.click(await within(form).findByRole('button', { name: 'Use' }));
    }
    expect(await screen.findByTestId('role-chat')).toHaveTextContent('qwen3.5:9b');
    // Clear asks first; Keep leaves the role alone.
    await user.click(screen.getByRole('button', { name: 'Clear the embeddings model' }));
    expect(
      within(role('Embeddings')).getByText('Stop using this model for embeddings?'),
    ).toBeInTheDocument();
    await user.click(within(role('Embeddings')).getByRole('button', { name: 'Keep' }));
    expect(screen.getByTestId('role-embedding')).toBeInTheDocument();
    expect(tauri.invoke).not.toHaveBeenCalledWith('clear_model_role', expect.anything());

    await user.click(screen.getByRole('button', { name: 'Clear the chat model' }));
    await user.click(within(role('Chat')).getByRole('button', { name: 'Clear' }));
    expect(
      await within(role('Chat')).findByText('Atlas no longer uses a model for chat.'),
    ).toBeInTheDocument();
    expect(within(role('Chat')).getByText('Not set.')).toBeInTheDocument();
    expect(screen.getByTestId('role-embedding')).toBeInTheDocument();

    tauri.setHandler('list_provider_profiles', () => []);
    await user.click(screen.getByRole('button', { name: 'Remove oMLX' }));
    await user.click(screen.getByRole('button', { name: 'Remove' }));
    expect(await within(role('Embeddings')).findByText('Not set.')).toBeInTheDocument();
  });

  it('offers an installed recommended model directly', async () => {
    const recs = tauri.recommendations();
    recs.embedding.models[0].installedAs = 'mlx-community/Qwen3-Embedding-0.6B-8bit';
    tauri.setHandler('get_model_recommendations', () => recs);
    tauri.setHandler('test_provider_connection', () =>
      tauri.connectionResult({
        modelIds: ['Qwen3.8-27B-MLX-4bit', 'mlx-community/Qwen3-Embedding-0.6B-8bit'],
      }),
    );
    const user = await openModels();
    await user.click(screen.getByRole('button', { name: 'Test oMLX' }));
    const rec = await screen.findByRole('group', { name: 'Recommended embeddings model' });
    expect(await within(rec).findByText(/is installed on oMLX/)).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith('get_model_recommendations', {
      listedModelIds: ['Qwen3.8-27B-MLX-4bit', 'mlx-community/Qwen3-Embedding-0.6B-8bit'],
    });
    await user.click(within(rec).getByRole('button', { name: 'Choose it' }));
    const form = screen.getByRole('form', { name: 'Choose the embeddings model' });
    expect(within(form).getByLabelText('Model')).toHaveValue(
      'mlx-community/Qwen3-Embedding-0.6B-8bit',
    );
  });

  it('labels where each service runs', async () => {
    await openModels([
      omlx,
      {
        ...omlx,
        id: 'p2',
        label: 'Studio',
        baseUrl: 'http://192.168.1.20:11434',
        location: 'another_computer',
      },
      {
        ...omlx,
        id: 'p3',
        label: 'OpenAI',
        baseUrl: 'https://api.openai.com/v1',
        location: 'internet',
      },
    ]);
    const cards = await screen.findAllByTestId('location');
    expect(cards.map((c) => c.textContent?.split('· ')[1])).toEqual([
      'This computer',
      'Another computer',
      'Internet service',
    ]);
    expect(
      screen.getByText(
        'An internet service: Atlas will ask before sending anything from your library to it.',
      ),
    ).toBeInTheDocument();
  });
});

describe('search', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const searches = () => tauri.invoke.mock.calls.filter(([c]) => c === 'search_documents');
  const lastSearch = () => (searches().at(-1)![1] as { query: Record<string, unknown> }).query;

  /** Launch, then search from Home's box. */
  async function searchFor(text: string, page = tauri.searchPage()) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () => page);
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), `${text}{Enter}`);
    await h1('Search');
    return user;
  }

  const results = [
    tauri.searchHit({ fileInstanceId: 'h1', why: 'both', otherLocations: 2 }),
    tauri.searchHit({
      fileInstanceId: 'h2',
      documentId: null,
      title: 'drive notes',
      titleSource: 'file_name',
      displayName: 'drive notes.epub',
      fileType: 'epub',
      location: null,
      pageNo: null,
      snippet: [],
      why: 'name',
      allWords: false,
      otherLocations: 1,
    }),
    tauri.searchHit({
      fileInstanceId: 'h3',
      title: 'Gears',
      displayName: 'gears.epub',
      fileType: 'epub',
      location: 'Chapter 3, part 2',
      availability: 'missing',
    }),
  ];
  const resultsPage = tauri.searchPage({ hits: results, matchedDocuments: 3 });

  it('is a main tab that shows only help before the first search', async () => {
    const user = await launchOpen();
    const nav = screen.getByRole('navigation', { name: 'Main' });
    expect(
      within(nav)
        .getAllByRole('button')
        .map((b) => b.textContent?.trim()),
    ).toEqual(['Home', 'Search', 'Library', 'Trails', 'Settings']);
    await user.click(within(nav).getByRole('button', { name: 'Search' }));
    await h1('Search');
    expect(screen.getByText(/Use quotes for an exact phrase/)).toBeInTheDocument();
    expect(screen.queryAllByTestId('search-hit')).toHaveLength(0);
    expect(searches()).toHaveLength(0);
  });

  it('Home describes the search and submits to the Search view', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    const user = await launchOpen();
    expect(
      screen.getByText(
        'Searches the text of your books and papers, and their names. Use quotes for an exact phrase. End a word with * to match its beginning.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/not AI search/)).not.toBeInTheDocument();
    await user.type(screen.getByLabelText('Search your library'), '"harmonic drive"{Enter}');
    await h1('Search');
    expect(lastSearch()).toEqual({
      text: '"harmonic drive"',
      sourceRootId: null,
      fileType: null,
      includeMissing: false,
      page: 0,
      pageSize: 20,
    });
    expect(screen.getByLabelText('Search your library')).toHaveValue('"harmonic drive"');
  });

  it('shows each result with where, a highlighted snippet and why it matched', async () => {
    await searchFor('harmonic drive', resultsPage);
    const hits = await screen.findAllByTestId('search-hit');
    expect(hits).toHaveLength(3);

    const [first, second, third] = hits.map((h) => within(h));
    expect(first.getByRole('heading', { name: 'Harmonic Drives' })).toBeInTheDocument();
    expect(first.getByText('Page 12')).toBeInTheDocument();
    expect(first.getByText('Books')).toBeInTheDocument();
    expect(first.getByText('harmonic.pdf')).toBeInTheDocument();
    const marks = hits[0].querySelectorAll('mark');
    expect([...marks].map((m) => m.textContent)).toEqual(['harmonic drive']);
    expect(first.getByTestId('snippet')).toHaveTextContent('… a harmonic drive has no backlash …');
    expect(first.getByText('In the text and the name')).toBeInTheDocument();
    expect(first.getByText('Also in 2 other places')).toBeInTheDocument();
    expect(first.queryByText('Some of your words')).not.toBeInTheDocument();

    // No title: the tidied file name is the heading, and the name isn't
    // repeated. Found by the any-words fallback.
    expect(second.getByRole('heading', { name: 'drive notes' })).toBeInTheDocument();
    expect(second.queryByText('drive notes.epub')).not.toBeInTheDocument();
    expect(second.getByText('In the name')).toBeInTheDocument();
    expect(second.getByText('Some of your words')).toBeInTheDocument();
    expect(second.getByText('Also in 1 other place')).toBeInTheDocument();
    expect(second.queryByTestId('snippet')).not.toBeInTheDocument();

    expect(third.getByText('Chapter 3, part 2')).toBeInTheDocument();
    expect(third.getByText('In the text')).toBeInTheDocument();
    expect(third.getByText('Missing')).toBeInTheDocument();
    expect(third.getByRole('button', { name: 'Open gears.epub' })).toBeDisabled();
    expect(screen.getByTestId('search-range')).toHaveTextContent('Showing 1–3 of 3 documents');
  });

  it('says when a result was found by meaning, never "Some of your words"', async () => {
    await searchFor(
      'a bored child underground',
      tauri.searchPage({
        hits: [
          tauri.searchHit({
            fileInstanceId: 'file-a',
            title: 'Alice',
            why: 'semantic',
            allWords: true,
            snippet: [
              { text: '… down the rabbit-hole, never once considering how ', hit: false },
              { text: 'underground', hit: true },
              { text: ' …', hit: false },
            ],
          }),
          tauri.searchHit({ fileInstanceId: 'file-b', title: 'Both ways', why: 'text_semantic' }),
        ],
        matchedDocuments: 2,
        semantic: { state: 'ready', coveragePercent: 100 },
      }),
    );
    const [first, second] = (await screen.findAllByTestId('search-hit')).map((h) => within(h));
    expect(first.getByText('Similar meaning')).toBeInTheDocument();
    expect(first.queryByText('Some of your words')).not.toBeInTheDocument();
    expect(first.getByText('underground').tagName).toBe('MARK');
    expect(second.getByText('In the text and similar meaning')).toBeInTheDocument();
    expect(screen.queryByTestId('meaning-note')).not.toBeInTheDocument();
  });

  it.each([
    [
      { state: 'ready', coveragePercent: 31 },
      'Meaning search covers 31% of files while the index builds.',
    ],
    [
      { state: 'unavailable', reason: 'unreachable' },
      "Showing text matches only: the AI service isn't reachable.",
    ],
    [
      { state: 'unavailable', reason: 'timeout' },
      'Showing text matches only: the AI service took too long. Search again in a moment.',
    ],
    [
      { state: 'unavailable', reason: 'loading' },
      'Showing text matches only: the meaning index is still loading.',
    ],
    [
      { state: 'unavailable', reason: 'not_permitted' },
      "Showing text matches only: embedding through an internet service isn't allowed in Settings › Privacy.",
    ],
  ] as const)('notes how meaning search went %#', async (semantic, text) => {
    await searchFor(
      'harmonic',
      tauri.searchPage({ hits: [tauri.searchHit()], matchedDocuments: 1, semantic }),
    );
    expect(await screen.findByTestId('meaning-note')).toHaveTextContent(text);
  });

  it('says nothing about meaning when no embedding model is set up', async () => {
    await searchFor(
      'harmonic',
      tauri.searchPage({ hits: [tauri.searchHit()], matchedDocuments: 1 }),
    );
    await screen.findAllByTestId('search-hit');
    expect(screen.queryByTestId('meaning-note')).not.toBeInTheDocument();
  });

  it('renders snippet text as text, never as markup', async () => {
    const hostile = tauri.searchHit({
      snippet: [
        { text: '<b>bold</b> <img src=x onerror="alert(1)"> ', hit: false },
        { text: '<i>drive</i>', hit: true },
      ],
    });
    await searchFor('drive', tauri.searchPage({ hits: [hostile], matchedDocuments: 1 }));
    const snippet = await screen.findByTestId('snippet');
    expect(snippet.querySelector('b, img, i')).toBeNull();
    expect(snippet).toHaveTextContent('<b>bold</b> <img src=x onerror="alert(1)"> <i>drive</i>');
    expect(snippet.querySelector('mark')?.textContent).toBe('<i>drive</i>');
  });

  it('filters and pages send the right query, with honest totals', async () => {
    const hits = Array.from({ length: 20 }, (_, i) =>
      tauri.searchHit({ fileInstanceId: `s${i}`, title: `Book ${i}` }),
    );
    const user = await searchFor('drive', tauri.searchPage({ hits, matchedDocuments: 45 }));
    expect(await screen.findByTestId('search-range')).toHaveTextContent(
      'Showing 1–20 of 45 documents',
    );
    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(lastSearch()).toMatchObject({ text: 'drive', page: 1 });
    await user.selectOptions(screen.getByRole('combobox', { name: 'Type' }), 'epub');
    expect(lastSearch()).toMatchObject({ fileType: 'epub', page: 0 });
    await user.selectOptions(screen.getByRole('combobox', { name: 'Folder' }), 'root-books');
    expect(lastSearch()).toMatchObject({ sourceRootId: 'root-books', fileType: 'epub' });

    tauri.setHandler('search_documents', () =>
      tauri.searchPage({ hits, matchedDocuments: 500, capped: true }),
    );
    await user.clear(screen.getByLabelText('Search your library'));
    await user.type(screen.getByLabelText('Search your library'), 'the{Enter}');
    await waitFor(() =>
      expect(screen.getByTestId('search-range')).toHaveTextContent(
        'Showing 1–20 of 500 or more documents',
      ),
    );
  });

  it('asks for a word when the query is only punctuation, without searching', async () => {
    await searchFor('"*" -- !!');
    expect(await screen.findByText('Type a word to search for.')).toBeInTheDocument();
    expect(searches()).toHaveLength(0);
  });

  it('says when nothing has been read yet', async () => {
    await searchFor('zebra', tauri.searchPage({ textReadFiles: 0, totalFiles: 12 }));
    expect(
      await screen.findByText("Atlas hasn't read any text yet. It searches file names meanwhile."),
    ).toBeInTheDocument();
    expect(screen.getByText('No file names match “zebra”.')).toBeInTheDocument();
  });

  it('says how much has been read when nothing matches', async () => {
    await searchFor('zebra', tauri.searchPage({ textReadFiles: 7, totalFiles: 10 }));
    expect(
      await screen.findByText('Nothing in your library mentions “zebra”.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Atlas has read the text of 7 of 10 files.')).toBeInTheDocument();
  });

  it('explains a search that fails', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () => {
      throw tauri.apiError('database_error', 'The library could not be read.', 'Restart Atlas.');
    });
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'drive{Enter}');
    expect(await screen.findByText("Atlas couldn't search your library")).toBeInTheDocument();
    expect(screen.getByText('Restart Atlas.')).toBeInTheDocument();
  });

  it('Home notes that reading continues', async () => {
    tauri.setHandler('get_extraction_status', () =>
      tauri.extractionOverview({
        active: tauri.extractionStatus(),
        counts: tauri.extractionCounts({
          done: 12_000,
          needsOcr: 400,
          pending: 17_606,
          total: 30_006,
        }),
      }),
    );
    await launchOpen();
    expect(await screen.findByTestId('reading-note')).toHaveTextContent(
      "Atlas is still reading text: 12,400 of 30,006 files so far. Search covers what's been read.",
    );
  });

  it('shows the reading note with results and refreshes them as text is read', async () => {
    await searchFor(
      'drive',
      tauri.searchPage({
        hits: [results[0]],
        matchedDocuments: 1,
        readingText: true,
        textReadFiles: 4,
        totalFiles: 10,
      }),
    );
    expect(await screen.findByTestId('reading-note')).toHaveTextContent(
      'Atlas is still reading text: 4 of 10 files so far.',
    );
    const before = searches().length;
    for (let i = 0; i < 4; i++) {
      events.emit(
        'extraction-status',
        tauri.extractionOverview({ counts: tauri.extractionCounts({ done: 5 + i, total: 10 }) }),
      );
    }
    await waitFor(() => expect(searches().length).toBe(before + 1), { timeout: 3000 });
  });

  it('opens a result, asking before downloading a cloud-only file', async () => {
    tauri.setHandler('open_original', (args) => {
      if (!args?.allowDownload) {
        throw tauri.apiError('file_cloud_only', 'This file is stored only in the cloud.');
      }
      return null;
    });
    const cloud = tauri.searchHit({ fileInstanceId: 'c1', storage: 'cloud_only' });
    const user = await searchFor('drive', tauri.searchPage({ hits: [cloud], matchedDocuments: 1 }));
    const hit = within(await screen.findByTestId('search-hit'));
    expect(hit.getByText('Only in the cloud')).toBeInTheDocument();
    await user.click(hit.getByRole('button', { name: 'Open harmonic.pdf' }));
    expect(
      await hit.findByText('This file is stored in the cloud. Opening it will download it.'),
    ).toBeInTheDocument();
    await user.click(hit.getByRole('button', { name: 'Download and open' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_original', {
      fileId: 'c1',
      allowDownload: true,
    });
    await user.click(hit.getByRole('button', { name: 'Show in Finder: harmonic.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('reveal_original', { fileId: 'c1' });
  });
});

describe('document details', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const row = tauri.fileInstance({
    id: 'f1',
    displayName: 'harmonic.pdf',
    displayPath: '/Users/test/Books/harmonic.pdf',
    sizeBytes: 4_200_000,
    documentId: 'doc-1',
    extraction: { state: 'done', reason: null },
  });

  async function openLibrary(detail: unknown) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_inventory_status', () => tauri.overview({ catalogedFiles: 1 }));
    tauri.setHandler('list_documents', () => tauri.documentPage({ items: [row], matchedSoFar: 1 }));
    tauri.setHandler('get_document_detail', () => detail);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await screen.findByRole('table');
    return user;
  }

  const dialog = () => screen.findByRole('dialog');
  const field = (d: HTMLElement, term: string) =>
    within(d).getByText(term, { selector: 'dt' }).nextElementSibling;

  it('opens from a Library row with everything Atlas knows, including other copies', async () => {
    const detail = tauri.documentDetail({
      file: row,
      authors: ['Ada Lin', 'Bo Okafor'],
      alsoAt: [
        tauri.fileInstance({
          id: 'f2',
          displayName: 'harmonic copy.pdf',
          displayPath: '/Users/test/Books/old/harmonic copy.pdf',
          documentId: 'doc-1',
        }),
        tauri.fileInstance({
          id: 'f3',
          displayName: 'harmonic (1).pdf',
          displayPath: '/Users/test/Books/harmonic (1).pdf',
          availability: 'missing',
          documentId: 'doc-1',
        }),
      ],
    });
    const user = await openLibrary(detail);
    await user.click(screen.getByRole('button', { name: 'Details for harmonic.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('get_document_detail', { fileId: 'f1' });
    const d = await dialog();
    expect(d).toHaveAccessibleName('Harmonic Drives');
    expect(within(d).getByText('harmonic.pdf')).toBeInTheDocument();
    expect(field(d, 'Authors')).toHaveTextContent('Ada Lin, Bo Okafor');
    expect(field(d, 'Language')).toHaveTextContent('English');
    expect(field(d, 'Pages')).toHaveTextContent('312');
    expect(field(d, 'Size')).toHaveTextContent('4.2 MB');
    expect(field(d, 'Folder')).toHaveTextContent('Books');
    expect(field(d, 'Path')).toHaveTextContent('/Users/test/Books/harmonic.pdf');
    expect(within(d).getByTestId('text-status')).toHaveTextContent('Text ready');

    const also = within(within(d).getByRole('region', { name: 'Also at' }));
    const copies = also.getAllByRole('listitem');
    expect(copies).toHaveLength(2);
    expect(within(copies[1]).getByText('Missing')).toBeInTheDocument();
    expect(also.getByRole('button', { name: 'Open harmonic (1).pdf' })).toBeDisabled();
    await user.click(also.getByRole('button', { name: 'Open harmonic copy.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_original', {
      fileId: 'f2',
      allowDownload: false,
    });
  });

  it('falls back to the file name and explains a file without text', async () => {
    const scanned = {
      ...row,
      extraction: { state: 'needs_ocr' as const, reason: 'no_text_layer' },
    };
    const user = await openLibrary(
      tauri.documentDetail({
        file: scanned,
        documentId: null,
        title: 'harmonic',
        titleSource: 'file_name',
        authors: [],
        language: null,
        pageCount: null,
        charCount: null,
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Details for harmonic.pdf' }));
    const d = await dialog();
    expect(d).toHaveAccessibleName('harmonic');
    expect(within(d).getByTestId('title-source')).toHaveTextContent('Title from the file name');
    expect(within(d).queryByText('Authors')).not.toBeInTheDocument();
    expect(within(d).queryByText('Pages')).not.toBeInTheDocument();
    expect(within(d).getByTestId('text-reason')).toHaveTextContent(
      "This PDF has no readable text layer. It looks scanned; Atlas can't read it without OCR.",
    );
    expect(within(d).queryByRole('region', { name: 'Also at' })).not.toBeInTheDocument();
  });

  it('says a file has not been read yet', async () => {
    const pending = {
      ...row,
      documentId: null,
      extraction: { state: 'pending' as const, reason: null },
    };
    const user = await openLibrary(
      tauri.documentDetail({
        file: pending,
        documentId: null,
        title: 'harmonic',
        titleSource: 'file_name',
        pageCount: null,
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Details for harmonic.pdf' }));
    expect(within(await dialog()).getByTestId('text-reason')).toHaveTextContent(
      "Atlas hasn't read this file yet.",
    );
  });

  it('closes with Escape or Close and returns focus to the row', async () => {
    const user = await openLibrary(tauri.documentDetail({ file: row }));
    const trigger = screen.getByRole('button', { name: 'Details for harmonic.pdf' });
    await user.click(trigger);
    await dialog();
    expect(screen.getByRole('button', { name: 'Close details' })).toHaveFocus();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(trigger).toHaveFocus();

    await user.keyboard('{Enter}');
    await dialog();
    await user.click(screen.getByRole('button', { name: 'Close details' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(trigger).toHaveFocus();
  });

  it('keeps Tab inside the open panel', async () => {
    const user = await openLibrary(tauri.documentDetail({ file: row }));
    await user.click(screen.getByRole('button', { name: 'Details for harmonic.pdf' }));
    const d = await dialog();
    for (let i = 0; i < 6; i++) {
      await user.tab();
      expect(d).toContainElement(document.activeElement as HTMLElement);
    }
  });

  it('shows loading, then a problem it can explain', async () => {
    let fail: (e: unknown) => void = () => {};
    const user = await openLibrary(null);
    tauri.setHandler('get_document_detail', () => new Promise((_, reject) => (fail = reject)));
    await user.click(screen.getByRole('button', { name: 'Details for harmonic.pdf' }));
    expect(within(await dialog()).getByText('Loading details…')).toBeInTheDocument();
    fail(tauri.apiError('file_not_found', 'Atlas no longer has this file in its library.'));
    expect(await screen.findByText("Atlas couldn't show this file's details")).toBeInTheDocument();
  });

  it('opens from a search result', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({ hits: [tauri.searchHit({ fileInstanceId: 'h1' })], matchedDocuments: 1 }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'drive{Enter}');
    await user.click(await screen.findByRole('button', { name: 'Details for harmonic.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('get_document_detail', { fileId: 'h1' });
    expect(await dialog()).toHaveAccessibleName('Harmonic Drives');
  });
});

describe('status bar with both jobs', () => {
  beforeEach(resetAll);

  it('shows cataloging and reading text at once, each with its own controls', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_inventory_status', () =>
      tauri.overview({ active: tauri.inventoryStatus({ state: 'running' }) }),
    );
    tauri.setHandler('get_extraction_status', () =>
      tauri.extractionOverview({
        active: tauri.extractionStatus(),
        counts: tauri.extractionCounts({ done: 3, pending: 7, total: 10 }),
      }),
    );
    const user = await launchOpen();
    const bar = within(screen.getByRole('contentinfo', { name: 'Status' }));
    expect(screen.getByTestId('inventory-status')).toBeInTheDocument();
    expect(screen.getByTestId('extraction-status')).toBeInTheDocument();
    expect(bar.getByRole('button', { name: 'Pause' })).toBeInTheDocument();
    await user.click(bar.getByRole('button', { name: 'Pause reading text' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pause_job', { jobId: 'extract-1' });
  });
});

describe('duplicates', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const dupCalls = () => tauri.invoke.mock.calls.filter(([c]) => c === 'list_duplicates');
  const lastDupQuery = () => (dupCalls().at(-1)![1] as { query: Record<string, unknown> }).query;

  async function openDuplicates(page = tauri.duplicatePage()) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_inventory_status', () => tauri.overview({ catalogedFiles: 4 }));
    tauri.setHandler('list_documents', () =>
      tauri.documentPage({ items: [tauri.fileInstance()], matchedSoFar: 1 }),
    );
    tauri.setHandler('list_duplicates', () => page);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await screen.findByRole('table');
    await user.click(screen.getByRole('radio', { name: 'Duplicates' }));
    return user;
  }

  const groups = [
    tauri.duplicateGroup(),
    tauri.duplicateGroup({
      documentId: 'doc-2',
      title: 'gear notes',
      titleSource: 'file_name',
      sizeBytes: 1_000_000,
      extraBytes: 1_000_000,
      copies: [
        tauri.fileInstance({ id: 'g1', displayName: 'gear_notes.pdf' }),
        tauri.fileInstance({ id: 'g2', displayName: 'gear_notes.pdf', storage: 'cloud_only' }),
        tauri.fileInstance({ id: 'g3', displayName: 'gear_notes.pdf', availability: 'missing' }),
      ],
    }),
  ];

  it('switches the Library to groups of identical copies, most extra space first', async () => {
    await openDuplicates(tauri.duplicatePage({ groups, totalGroups: 2, extraBytes: 9_400_000 }));
    expect(lastDupQuery()).toEqual({
      sourceRootId: null,
      fileType: null,
      includeMissing: false,
      page: 0,
      pageSize: 20,
    });
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText('Search file names and folders')).toBeNull();
    expect(screen.queryByRole('combobox', { name: 'Text status' })).toBeNull();
    expect(screen.getByTestId('duplicates-summary')).toHaveTextContent(
      '2 groups · 9.4 MB in extra copies',
    );
    expect(screen.getByText(/never deletes or moves your files/)).toBeInTheDocument();
    const [first, second] = screen.getAllByTestId('duplicate-group').map((g) => within(g));
    expect(first.getByRole('heading', { name: 'Harmonic Drives' })).toBeInTheDocument();
    expect(first.getByText('3 copies')).toBeInTheDocument();
    expect(first.getByText('4.2 MB each')).toBeInTheDocument();
    expect(first.getByText('8.4 MB in extra copies')).toBeInTheDocument();
    expect(first.getAllByRole('listitem')).toHaveLength(3);
    // The missing copy is listed but not counted; the source of the title
    // is said.
    expect(second.getByText('2 copies')).toBeInTheDocument();
    expect(second.getByText('Title from the file name')).toBeInTheDocument();
    expect(second.getByText('Missing')).toBeInTheDocument();
    expect(second.getByText('Only in the cloud')).toBeInTheDocument();
    expect(screen.getByTestId('duplicates-range')).toHaveTextContent('Showing 1–2 of 2 groups');
  });

  it('filters, pages and opens copies', async () => {
    const user = await openDuplicates(
      tauri.duplicatePage({ groups, totalGroups: 45, extraBytes: 9_400_000 }),
    );
    await user.selectOptions(screen.getByRole('combobox', { name: 'Type' }), 'pdf');
    expect(lastDupQuery()).toMatchObject({ fileType: 'pdf', page: 0 });
    await user.click(screen.getByRole('checkbox', { name: 'Include missing files' }));
    expect(lastDupQuery()).toMatchObject({ includeMissing: true });
    await user.click(screen.getByRole('button', { name: 'Next' }));
    expect(lastDupQuery()).toMatchObject({ page: 1 });
    const group = within(screen.getAllByTestId('duplicate-group')[0]);
    await user.click(group.getAllByRole('button', { name: 'Open harmonic.pdf' })[1]);
    expect(tauri.invoke).toHaveBeenCalledWith('open_original', {
      fileId: 'c3',
      allowDownload: false,
    });
    await user.click(group.getByRole('button', { name: 'Details for harmonic copy.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('get_document_detail', { fileId: 'c2' });
  });

  it('explains when there are no duplicates, and goes back to files', async () => {
    const user = await openDuplicates();
    expect(await screen.findByText('No duplicates found.')).toBeInTheDocument();
    expect(screen.getByText(/finds copies by their contents/)).toBeInTheDocument();
    await user.click(screen.getByRole('radio', { name: 'Files' }));
    expect(await screen.findByRole('table')).toBeInTheDocument();
  });

  it('explains a failure', async () => {
    tauri.setHandler('list_duplicates', () => {
      throw tauri.apiError('database_error', 'The library could not be read.', 'Restart Atlas.');
    });
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await user.click(await screen.findByRole('radio', { name: 'Duplicates' }));
    expect(await screen.findByText("Atlas couldn't list duplicates")).toBeInTheDocument();
  });
});

describe('storage', () => {
  beforeEach(resetAll);

  async function openStorage() {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const heading = await screen.findByRole('heading', { level: 2, name: 'Storage' });
    return { user, section: within(heading.closest('section')!) };
  }

  it('shows usage by category and the estimate as a range', async () => {
    const { section } = await openStorage();
    const panel = within(await section.findByTestId('storage-usage'));
    expect(await panel.findByText('620 MB')).toBeInTheDocument();
    expect(panel.getByText('Text Atlas has read')).toBeInTheDocument();
    expect(panel.getByText('250 MB')).toBeInTheDocument();
    expect(panel.getByText('530 MB')).toBeInTheDocument();
    expect(panel.getByText('12.0 MB')).toBeInTheDocument();
    expect(panel.getByTestId('indexing-estimate')).toHaveTextContent(
      'Text for the remaining 17,600 files will need about 1.5–2.5 GB.',
    );
  });

  it.each([
    [{ kind: 'not_enough_read', done: 12, needed: 50 } as const, /Not enough read yet/],
    [{ kind: 'all_read' } as const, 'Atlas has read the text of every file it can.'],
    [
      {
        kind: 'range',
        remainingFiles: 30_000,
        lowBytes: 800_000_000,
        highBytes: 1_200_000_000,
      } as const,
      'Text for the remaining 30,000 files will need about 800 MB–1.2 GB. That may be more than the free space on this drive.',
    ],
  ])('estimate %#', async (estimate, text) => {
    tauri.setHandler('get_storage_usage', () =>
      tauri.storageUsage({ estimate, dataVolumeFreeBytes: 1_000_000_000 }),
    );
    const { section } = await openStorage();
    expect(await section.findByTestId('indexing-estimate')).toHaveTextContent(text);
  });

  it('says it is measuring until the numbers arrive', async () => {
    tauri.setHandler('get_storage_usage', () => new Promise(() => {}));
    const { section } = await openStorage();
    expect(await section.findByText('Measuring…')).toBeInTheDocument();
    expect(section.getByRole('button', { name: 'Clean caches' })).toBeDisabled();
  });

  it('cleans caches and says what that freed', async () => {
    tauri.setHandler('clean_caches', () => ({
      freedBytes: 12_000_000,
      usage: tauri.storageUsage({ cacheBytes: 0 }),
    }));
    const { user, section } = await openStorage();
    await user.click(await section.findByRole('button', { name: 'Clean caches' }));
    expect(tauri.invoke).toHaveBeenCalledWith('clean_caches', undefined);
    expect(await section.findByTestId('clean-result')).toHaveTextContent('Freed 12.0 MB.');
    expect(section.getByText('0 bytes')).toBeInTheDocument();
    tauri.setHandler('clean_caches', () => ({ freedBytes: 0, usage: tauri.storageUsage() }));
    await user.click(section.getByRole('button', { name: 'Clean caches' }));
    await waitFor(() =>
      expect(section.getByTestId('clean-result')).toHaveTextContent('Nothing to clean.'),
    );
  });

  it('explains a measuring failure', async () => {
    tauri.setHandler('get_storage_usage', () => {
      throw tauri.apiError('database_error', 'The library could not be read.');
    });
    const { section } = await openStorage();
    expect(await section.findByText("Atlas couldn't measure its storage")).toBeInTheDocument();
  });

  it('shows your trails and notes and their snapshots apart from the caches', async () => {
    const { section } = await openStorage();
    expect(await section.findByText('Your trails and notes')).toBeInTheDocument();
    expect(section.getByText('Snapshots (2)')).toBeInTheDocument();
    expect(section.getByTestId('knowledge-bytes')).toHaveTextContent(/KB/);
    expect(section.getByTestId('snapshot-bytes')).toHaveTextContent(/KB/);
    expect(section.queryByTestId('knowledge-note')).not.toBeInTheDocument();
    expect(section.getByText(/your trails and notes and their\s+snapshots/)).toBeInTheDocument();
  });

  it.each([
    [{ state: 'damaged' } as const, "Atlas can't read your trails and notes"],
    [{ state: 'incompatible' } as const, 'saved by a newer version of Atlas'],
    [
      {
        snapshots: { count: 2, bytes: 1, newestAt: null, lastOutcome: 'low_space' },
      } as const,
      "didn't make a new snapshot",
    ],
    [
      { snapshots: { count: 2, bytes: 1, newestAt: null, lastOutcome: 'failed' } } as const,
      'failed its check and was removed',
    ],
  ])('says what happened to your trails and notes %#', async (status, text) => {
    tauri.setHandler('get_knowledge_status', () => tauri.knowledgeStatus(status));
    const { section } = await openStorage();
    expect(await section.findByTestId('knowledge-note')).toHaveTextContent(text);
  });

  it('still shows the sizes when the store status cannot be read', async () => {
    tauri.setHandler('get_knowledge_status', () => {
      throw tauri.apiError('knowledge_unavailable', "Atlas couldn't read your trails and notes.");
    });
    const { section } = await openStorage();
    expect(await section.findByText('Your trails and notes')).toBeInTheDocument();
    expect(section.queryByTestId('knowledge-note')).not.toBeInTheDocument();
  });
});

describe('title sources', () => {
  beforeEach(resetAll);

  it('says when a title comes from the first page', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({
        hits: [tauri.searchHit({ title: 'Strain Wave Gearing', titleSource: 'contents' })],
        matchedDocuments: 1,
      }),
    );
    tauri.setHandler('get_document_detail', () =>
      tauri.documentDetail({ title: 'Strain Wave Gearing', titleSource: 'contents' }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'gear{Enter}');
    const hit = within(await screen.findByTestId('search-hit'));
    expect(hit.getByRole('heading', { name: 'Strain Wave Gearing' })).toBeInTheDocument();
    expect(hit.getByText('harmonic.pdf')).toBeInTheDocument();
    await user.click(hit.getByRole('button', { name: 'Details for harmonic.pdf' }));
    const d = await screen.findByRole('dialog');
    expect(within(d).getByTestId('title-source')).toHaveTextContent('Title from the first page');
  });
});

describe('indexing meaning', () => {
  beforeEach(resetAll);

  const model = {
    name: 'Qwen3-Embedding-0.6B-8bit',
    dims: 1024,
    profileLabel: 'oMLX',
    location: 'this_computer',
  } as const;
  const counts = tauri.embeddingCounts({ indexed: 3, waiting: 7, total: 10 });
  const withModel = (o: Partial<ReturnType<typeof tauri.embeddingOverview>> = {}) =>
    tauri.embeddingOverview({ model, needsModel: false, hasAiService: true, counts, ...o });

  async function launchWith(ov: ReturnType<typeof tauri.embeddingOverview>) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_embedding_status', () => ov);
    return launchOpen();
  }

  const line = () => screen.getByTestId('embedding-status');

  it.each([
    [{ state: 'running' }, {}, 'Indexing meaning · 3 of 10 files'],
    [{ state: 'running' }, { reading: true }, 'Indexing meaning · 3 files so far'],
    [
      { state: 'running' },
      { rebuilding: true },
      'Rebuilding meaning index · 3 of 10 files · search uses the previous index',
    ],
    [
      { state: 'paused_policy', pauseReason: 'on_battery' },
      {},
      'Paused indexing meaning · On battery',
    ],
    [
      { state: 'paused_policy', pauseReason: 'battery_low' },
      {},
      'Paused indexing meaning · Resumes at 25% or when plugged in',
    ],
    [{ state: 'paused_user' }, {}, 'Paused indexing meaning by you · 3 of 10 files'],
    [{ state: 'queued' }, {}, 'Waiting to index meaning · 3 of 10 files'],
    [
      { state: 'failed_retryable', lastError: 'provider_unreachable' },
      {},
      "Can't index meaning · AI service not reachable",
    ],
  ] as const)('status line %#', async (active, extra, text) => {
    await launchWith(withModel({ active: tauri.embeddingStatus(active), ...extra }));
    expect(line()).toHaveTextContent(text);
  });

  it('is up to date when every file is indexed', async () => {
    await launchWith(withModel({ counts: tauri.embeddingCounts({ indexed: 10, total: 10 }) }));
    expect(line()).toHaveTextContent('Meaning up to date · 10 files');
    expect(screen.queryByRole('button', { name: 'Pause indexing meaning' })).toBeNull();
  });

  it('says nothing about meaning to someone without an AI service', async () => {
    await launchWith(tauri.embeddingOverview({ counts: tauri.embeddingCounts({ total: 10 }) }));
    expect(screen.queryByTestId('embedding-status')).toBeNull();
  });

  it('asks for a model or for permission, with a way to Settings', async () => {
    const user = await launchWith(
      tauri.embeddingOverview({ hasAiService: true, counts: tauri.embeddingCounts({ total: 10 }) }),
    );
    expect(line()).toHaveTextContent('Indexing meaning needs an embedding model');
    await user.click(screen.getByRole('button', { name: 'Open Settings' }));
    await h1('Settings');
    events.emit('embedding-status', withModel({ needsPermission: true }));
    await waitFor(() => expect(line()).toHaveTextContent('Indexing meaning needs your permission'));
  });

  it('pauses, resumes, cancels and retries the meaning job only', async () => {
    const user = await launchWith(withModel({ active: tauri.embeddingStatus() }));
    await user.click(screen.getByRole('button', { name: 'Pause indexing meaning' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pause_job', { jobId: 'embed-1' });
    events.emit(
      'embedding-status',
      withModel({ active: tauri.embeddingStatus({ state: 'paused_user' }) }),
    );
    await user.click(await screen.findByRole('button', { name: 'Resume indexing meaning' }));
    expect(tauri.invoke).toHaveBeenCalledWith('resume_job', { jobId: 'embed-1' });
    await user.click(screen.getByRole('button', { name: 'Cancel indexing meaning' }));
    expect(tauri.invoke).toHaveBeenCalledWith('cancel_job', { jobId: 'embed-1' });
    events.emit(
      'embedding-status',
      withModel({
        active: tauri.embeddingStatus({
          state: 'failed_retryable',
          lastError: 'provider_unreachable',
        }),
      }),
    );
    await user.click(await screen.findByRole('button', { name: 'Retry indexing meaning' }));
    expect(tauri.invoke).toHaveBeenCalledWith('resume_job', { jobId: 'embed-1' });
    expect(tauri.invoke).not.toHaveBeenCalledWith('pause_job', { jobId: 'extract-1' });
  });

  it('shows a folder line, Index meaning now, and Home’s note', async () => {
    const user = await launchWith(
      withModel({
        active: tauri.embeddingStatus(),
        roots: [{ sourceRootId: 'root-books', counts, activeJobId: null }],
      }),
    );
    expect(screen.getByTestId('meaning-note')).toHaveTextContent(
      "Atlas is still indexing meaning: 3 of 10 files. Meaning search covers what's been indexed.",
    );
    expect(screen.getByTestId('root-meaning-line')).toHaveTextContent(
      'Meaning: 3 indexed · 7 waiting',
    );
    await user.click(screen.getByRole('button', { name: /Index meaning now in/ }));
    expect(tauri.invoke).toHaveBeenCalledWith('start_embedding', { sourceRootId: 'root-books' });
  });

  it('lets the owner allow embedding through an internet service, showing the volume', async () => {
    tauri.setHandler('get_embedding_status', () =>
      withModel({ needsPermission: true, counts: tauri.embeddingCounts({ total: 29_870 }) }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const box = await screen.findByRole('checkbox', {
      name: 'Allow embedding my library through an internet service',
    });
    expect(box).not.toBeChecked();
    expect(await screen.findByText(/about 620 MB of text from 29,870 files/)).toBeInTheDocument();
    await user.click(box);
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: {
          appearance: { theme: 'system' },
          privacy: {
            allowRemoteEmbedding: true,
            disableExternalAi: false,
            acknowledged: [],
            recordTrails: true,
            expireTrails: true,
          },
        },
      }),
    );
    // A theme saved afterwards keeps the privacy choice.
    await user.click(screen.getByRole('radio', { name: 'Dark' }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: {
          appearance: { theme: 'dark' },
          privacy: {
            allowRemoteEmbedding: true,
            disableExternalAi: false,
            acknowledged: [],
            recordTrails: true,
            expireTrails: true,
          },
        },
      }),
    );
  });

  it('turns all external AI off, which also locks the embedding choice', async () => {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    expect(
      await screen.findByText(/Ask Atlas sends your question and up to 16 short excerpts/),
    ).toBeInTheDocument();
    const off = screen.getByRole('checkbox', { name: 'Disable all external AI' });
    expect(off).not.toBeChecked();
    await user.click(off);
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: {
          appearance: { theme: 'system' },
          privacy: {
            allowRemoteEmbedding: false,
            disableExternalAi: true,
            acknowledged: [],
            recordTrails: true,
            expireTrails: true,
          },
        },
      }),
    );
    expect(
      await screen.findByRole('checkbox', {
        name: 'Allow embedding my library through an internet service',
      }),
    ).toBeDisabled();
    expect(screen.getByRole('checkbox', { name: 'Disable all external AI' })).toBeChecked();
  });

  it('lists the internet services agreed to for Ask Atlas, each one forgettable', async () => {
    const agreed = {
      profileId: 'cloud',
      baseUrl: 'https://api.example.com/v1',
      label: 'Example Cloud',
      at: '2026-11-10T09:00:00Z',
    };
    tauri.setHandler('get_settings', () => ({
      appearance: { theme: 'system' },
      privacy: { allowRemoteEmbedding: false, disableExternalAi: false, acknowledged: [agreed] },
    }));
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const row = await screen.findByTestId('agreed-service');
    expect(row).toHaveTextContent('Example Cloud');
    expect(row).toHaveTextContent('https://api.example.com/v1');
    await user.click(screen.getByRole('button', { name: 'Forget agreement for Example Cloud' }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: {
          appearance: { theme: 'system' },
          privacy: {
            allowRemoteEmbedding: false,
            disableExternalAi: false,
            acknowledged: [],
            recordTrails: true,
            expireTrails: true,
          },
        },
      }),
    );
  });

  it('shows the Meaning index in Storage, with its estimate', async () => {
    tauri.setHandler('get_storage_usage', () =>
      tauri.storageUsage({
        meaningCatalogBytes: 300_000_000,
        meaningFileBytes: 100_000_000,
        meaningEstimate: {
          remainingFiles: 17_600,
          lowBytes: 800_000_000,
          highBytes: 1_300_000_000,
        },
      }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByTestId('meaning-bytes')).toHaveTextContent('400 MB');
    expect(screen.getByTestId('meaning-estimate')).toHaveTextContent(
      'Meaning for the remaining 17,600 files will need about 800 MB–1.3 GB.',
    );
    expect(
      screen.getByText(/never removes your catalog, the text Atlas has read, the meaning index/),
    ).toBeInTheDocument();
  });
});

describe('related documents', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const row = tauri.fileInstance({
    id: 'f1',
    displayName: 'harmonic.pdf',
    documentId: 'doc-1',
    extraction: { state: 'done', reason: null },
  });
  const withModel = tauri.embeddingOverview({ needsModel: false, hasAiService: true });
  const near = tauri.relatedHit();
  const far = tauri.relatedHit({
    documentId: 'doc-3',
    fileInstanceId: 'file-3',
    title: 'Gear Trains',
    displayName: 'gears.epub',
    fileType: 'epub',
    location: null,
    availability: 'missing',
  });

  async function openDetails(related: unknown) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_inventory_status', () => tauri.overview({ catalogedFiles: 1 }));
    tauri.setHandler('get_embedding_status', () => withModel);
    tauri.setHandler('list_documents', () => tauri.documentPage({ items: [row], matchedSoFar: 1 }));
    tauri.setHandler('get_document_detail', (args) =>
      args?.fileId === 'file-2'
        ? tauri.documentDetail({
            file: tauri.fileInstance({ id: 'file-2', documentId: 'doc-2' }),
            documentId: 'doc-2',
            title: 'Strain Wave Gearing',
          })
        : tauri.documentDetail({ file: row }),
    );
    tauri.setHandler('get_related_documents', () => related);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await screen.findByRole('table');
    await user.click(screen.getByRole('button', { name: 'Details for harmonic.pdf' }));
    return { user, d: await screen.findByRole('dialog') };
  }

  const relatedRegion = async (d: HTMLElement) =>
    within(await within(d).findByRole('region', { name: 'Related' }));

  it('lists documents close in meaning with where they match and their actions', async () => {
    const { user, d } = await openDetails(
      tauri.relatedPage({ status: 'ready', hits: [near, far] }),
    );
    const related = await relatedRegion(d);
    expect(tauri.invoke).toHaveBeenCalledWith('get_related_documents', {
      documentId: 'doc-1',
      limit: 10,
    });
    const items = related.getAllByTestId('related-hit');
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent('Strain Wave Gearing');
    expect(items[0]).toHaveTextContent('PDF · Books');
    expect(items[0]).toHaveTextContent('Related by meaning: Page 40');
    expect(within(items[1]).getByText('Related by meaning')).toBeInTheDocument();
    expect(within(items[1]).getByText('Missing')).toBeInTheDocument();
    expect(related.getByRole('button', { name: 'Open gears.epub' })).toBeDisabled();
    await user.click(related.getByRole('button', { name: 'Open strain-wave.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith('open_original', {
      fileId: 'file-2',
      allowDownload: false,
    });
  });

  it('shows a related document in the panel, with Back, and Close returns focus', async () => {
    const { user, d } = await openDetails(tauri.relatedPage({ status: 'ready', hits: [near] }));
    const related = await relatedRegion(d);
    await user.click(related.getByRole('button', { name: 'Details for Strain Wave Gearing' }));
    await waitFor(() => expect(d).toHaveAccessibleName('Strain Wave Gearing'));
    expect(tauri.invoke).toHaveBeenCalledWith('get_document_detail', { fileId: 'file-2' });
    await user.click(within(d).getByRole('button', { name: 'Back to Harmonic Drives' }));
    await waitFor(() => expect(d).toHaveAccessibleName('Harmonic Drives'));
    expect(within(d).queryByRole('button', { name: /^Back to/ })).toBeNull();
    await user.click(within(d).getByRole('button', { name: 'Close details' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: 'Details for harmonic.pdf' })).toHaveFocus();
  });

  it.each([
    ['not_indexed', 'Not indexed for meaning yet.'],
    ['no_text', 'Atlas found no text in this file to compare.'],
    ['loading', 'Finding related documents…'],
    ['failed', "Atlas couldn't find related documents right now."],
    ['ready', 'Nothing else in your library is close in meaning.'],
  ] as const)('says why there is no list: %s', async (status, text) => {
    const { d } = await openDetails(tauri.relatedPage({ status }));
    expect(await within(d).findByText(text)).toBeInTheDocument();
  });

  it('shows nothing without an embedding model', async () => {
    const { d } = await openDetails(tauri.relatedPage({ status: 'off' }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('get_related_documents', expect.anything()),
    );
    expect(within(d).queryByRole('region', { name: 'Related' })).toBeNull();
  });

  it('asks again when the meaning index moves on', async () => {
    let answer = tauri.relatedPage({ status: 'not_indexed' });
    const { d } = await openDetails(answer);
    tauri.setHandler('get_related_documents', () => answer);
    expect(await within(d).findByText('Not indexed for meaning yet.')).toBeInTheDocument();
    answer = tauri.relatedPage({ status: 'ready', hits: [near] });
    events.emit('embedding-status', withModel);
    expect(await within(d).findByText('Strain Wave Gearing')).toBeInTheDocument();
  });

  it('opens Related from a search result with More like this', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('get_embedding_status', () => withModel);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({
        hits: [tauri.searchHit({ fileInstanceId: 'f1', why: 'semantic' })],
        matchedDocuments: 1,
        semantic: { state: 'ready', coveragePercent: 100 },
      }),
    );
    tauri.setHandler('get_document_detail', () => tauri.documentDetail({ file: row }));
    tauri.setHandler('get_related_documents', () =>
      tauri.relatedPage({ status: 'ready', hits: [near] }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'gears{Enter}');
    const hit = within(await screen.findByTestId('search-hit'));
    await user.click(hit.getByRole('button', { name: 'More like this: harmonic.pdf' }));
    const d = await screen.findByRole('dialog');
    const heading = await within(d).findByRole('heading', { name: 'Related' });
    await waitFor(() => expect(heading).toHaveFocus());
  });

  it('offers More like this only when meaning search is set up', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({ hits: [tauri.searchHit()], matchedDocuments: 1 }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'gears{Enter}');
    const hit = within(await screen.findByTestId('search-hit'));
    expect(hit.queryByRole('button', { name: /More like this/ })).toBeNull();
  });
});

describe('Ask Atlas', () => {
  beforeEach(resetAll);

  type Step = import('./lib/api/bindings/AnswerStep').AnswerStep;
  const step = (s: Step, answerId = 'answer-1') =>
    events.emit('answer-stream', { answerId, step: s });
  const asks = () => tauri.invoke.mock.calls.filter(([c]) => c === 'ask_atlas');

  async function openAsk(reply: unknown = tauri.askStarted()) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('ask_atlas', () => reply);
    tauri.setHandler('get_document_detail', (args) =>
      tauri.documentDetail({
        file: tauri.fileInstance({ id: String(args?.fileId), documentId: 'doc-1' }),
      }),
    );
    const user = await launchOpen();
    const nav = screen.getByRole('navigation', { name: 'Main' });
    await user.click(within(nav).getByRole('button', { name: 'Search' }));
    await h1('Search');
    await user.click(screen.getByRole('radio', { name: 'Ask Atlas' }));
    return user;
  }

  async function ask(user: ReturnType<typeof userEvent.setup>, q = 'how do harmonic drives work') {
    await user.type(screen.getByLabelText('Ask your library'), q);
    await user.click(screen.getByRole('button', { name: 'Ask' }));
  }

  /** The answer's paragraphs as text (chips read as their numbers). */
  const answerText = () =>
    [...screen.getByRole('article', { name: 'Answer' }).querySelectorAll('p')]
      .map((p) => p.textContent)
      .join('\n');

  it('is the Search screen’s second mode, a radio group beside Find sources', async () => {
    const user = await openAsk();
    const group = screen.getByRole('group', { name: 'Search mode' });
    expect(
      within(group)
        .getAllByRole('radio')
        .map((r) => r.closest('label')?.textContent?.trim()),
    ).toEqual(['Find sources', 'Ask Atlas']);
    expect(screen.getByRole('radio', { name: 'Ask Atlas' })).toBeChecked();
    expect(screen.getByLabelText('Ask your library')).toBeInTheDocument();
    expect(screen.queryByLabelText('Search your library')).toBeNull();
    expect(screen.getByRole('button', { name: 'Ask' })).toBeDisabled();
    await user.click(screen.getByRole('radio', { name: 'Find sources' }));
    expect(screen.getByLabelText('Search your library')).toBeInTheDocument();
  });

  it('asks with the folder and type filters, which don’t search while asking', async () => {
    const user = await openAsk();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Type' }), 'epub');
    expect(tauri.invoke.mock.calls.filter(([c]) => c === 'search_documents')).toHaveLength(0);
    await ask(user, '  why hired soldiers fail  ');
    expect(asks()).toEqual([
      [
        'ask_atlas',
        { query: { question: 'why hired soldiers fail', sourceRootId: null, fileType: 'epub' } },
      ],
    ]);
  });

  it('streams the answer with a named chip for each citation, then says it once', async () => {
    const user = await openAsk();
    await ask(user);
    expect(await screen.findByTestId('answer-status')).toHaveTextContent('Waiting for the answer…');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    step({ kind: 'thinking' });
    await waitFor(() => expect(screen.getByTestId('answer-status')).toHaveTextContent('Thinking…'));
    step({ kind: 'delta', text: 'A flexible spline gives high reduction [1]' });
    step({ kind: 'citation', n: 1 });
    step({ kind: 'delta', text: ', invented in 1957 [2].' });
    step({ kind: 'citation', n: 2 });
    await waitFor(() => expect(answerText()).toContain('invented in 1957'));
    expect(screen.queryByTestId('answer-status')).toBeNull();
    expect(screen.getByTestId('answer-announcer')).toHaveTextContent('Atlas is answering.');
    expect(
      screen.getByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' }),
    ).toHaveTextContent('1');
    expect(
      screen.getByRole('button', { name: 'Source 2: Strain Wave Gearing' }),
    ).toBeInTheDocument();
    step({ kind: 'done', cited: [1, 2], dropped: 0 });
    await waitFor(() =>
      expect(screen.getByTestId('answer-announcer')).toHaveTextContent(
        'Answer: A flexible spline gives high reduction (source 1), invented in 1957 (source 2).',
      ),
    );
    expect(screen.getByRole('button', { name: 'Ask' })).toBeInTheDocument();
    expect(screen.queryByTestId('dropped-note')).toBeNull();
  });

  it('keeps the steps that arrive before Atlas says the answer started', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    const user = await openAsk();
    tauri.setHandler('ask_atlas', () => {
      step({ kind: 'delta', text: 'Early words [1].' });
      step({ kind: 'delta', text: 'Not ours.' }, 'answer-other');
      return tauri.askStarted();
    });
    await ask(user);
    await waitFor(() => expect(answerText()).toBe('Early words 1.'));
    step({ kind: 'done', cited: [1], dropped: 0 });
    expect(await screen.findByRole('button', { name: 'Ask' })).toBeInTheDocument();
  });

  it('opens a citation’s document with the passage sent; Close returns to the chip', async () => {
    const user = await openAsk();
    await ask(user);
    step({ kind: 'delta', text: 'High reduction [1].' });
    step({ kind: 'done', cited: [1], dropped: 0 });
    const chip = await screen.findByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' });
    await user.click(chip);
    const d = await screen.findByRole('dialog');
    const cited = within(await within(d).findByTestId('cited-passage'));
    expect(cited.getByRole('heading', { name: 'Cited passage' })).toBeInTheDocument();
    expect(cited.getByText('Source 1')).toBeInTheDocument();
    expect(cited.getByText('Page 12')).toBeInTheDocument();
    expect(cited.getByText(/flexible spline to reach high reduction/)).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith('get_document_detail', { fileId: 'file-1' });
    await user.click(within(d).getByRole('button', { name: 'Close details' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(chip).toHaveFocus();
  });

  it('lists every source sent, cited or not, each with its passage', async () => {
    const user = await openAsk();
    await ask(user);
    step({ kind: 'delta', text: 'Only the first [1].' });
    step({ kind: 'done', cited: [1], dropped: 0 });
    const region = within(await screen.findByRole('region', { name: 'Sources used (2)' }));
    const items = region.getAllByTestId('source-used');
    expect(items).toHaveLength(2);
    expect(within(items[0]).getByText('Cited')).toBeInTheDocument();
    expect(within(items[1]).queryByText('Cited')).toBeNull();
    expect(items[1]).toHaveTextContent('Strain wave gearing was invented in 1957.');
    await user.click(
      region.getByRole('button', { name: 'Details for Source 2: Strain Wave Gearing' }),
    );
    const d = await screen.findByRole('dialog');
    expect(
      await within(d).findByText('Strain wave gearing was invented in 1957.'),
    ).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith('get_document_detail', { fileId: 'file-2' });
  });

  it('says when Atlas found little, when references were removed, and when meaning was off', async () => {
    const user = await openAsk(
      tauri.askStarted({ weak: true, semantic: { state: 'unavailable', reason: 'unreachable' } }),
    );
    await ask(user);
    expect(await screen.findByTestId('weak-note')).toHaveTextContent(
      'Atlas found little about this in your library; the answer may be thin.',
    );
    expect(screen.getByTestId('ask-meaning-note')).toHaveTextContent(
      "Passages were found by their words only: the AI service isn't reachable.",
    );
    step({ kind: 'delta', text: 'Thin [1].' });
    step({ kind: 'done', cited: [1], dropped: 1 });
    expect(await screen.findByTestId('dropped-note')).toHaveTextContent(
      "One reference the model gave didn't match any source and was removed.",
    );
    await ask(user, ' again');
    step({ kind: 'delta', text: 'Thin [1].' });
    step({ kind: 'done', cited: [1], dropped: 3 });
    expect(await screen.findByTestId('dropped-note')).toHaveTextContent(
      "3 references the model gave didn't match any source and were removed.",
    );
  });

  it('says when the library has nothing on it, without asking the model', async () => {
    const user = await openAsk({
      kind: 'nothing_found',
      semantic: { state: 'ready', coveragePercent: 100 },
    });
    await ask(user);
    expect(await screen.findByTestId('answer-nothing')).toHaveTextContent(
      'Atlas found nothing about this in your library, so it didn’t ask the model.',
    );
    expect(screen.queryByRole('region', { name: /Sources used/ })).toBeNull();
  });

  it('shows what an internet service would receive and sends only after Continue', async () => {
    const needed = {
      kind: 'needs_disclosure',
      profileId: 'p-net',
      profileLabel: 'Cloud Chat',
      baseUrl: 'https://chat.example/v1',
      excerptCount: 14,
      bytes: 23_400,
    };
    const user = await openAsk(needed);
    await ask(user);
    const d = await screen.findByRole('dialog', { name: 'Send excerpts to an internet service?' });
    expect(d).toHaveAccessibleDescription(
      "Atlas will send your question and 14 excerpts from your library, about 23.4 KB, to Cloud Chat at https://chat.example/v1. It's an internet service: what you send leaves this computer.",
    );
    expect(within(d).getByRole('button', { name: 'Continue' })).toHaveFocus();
    await user.click(within(d).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(tauri.invoke.mock.calls.filter(([c]) => c === 'acknowledge_disclosure')).toHaveLength(0);
    expect(screen.getByLabelText('Ask your library')).toHaveFocus();

    await user.click(screen.getByRole('button', { name: 'Ask' }));
    const again = await screen.findByRole('dialog');
    tauri.setHandler('ask_atlas', () => tauri.askStarted());
    await user.click(within(again).getByRole('button', { name: 'Continue' }));
    expect(tauri.invoke).toHaveBeenCalledWith('acknowledge_disclosure', { profileId: 'p-net' });
    await waitFor(() => expect(asks()).toHaveLength(3));
    expect(await screen.findByRole('region', { name: 'Sources used (2)' })).toBeInTheDocument();
  });

  it('stops an answer and keeps what had arrived', async () => {
    const user = await openAsk();
    await ask(user);
    step({ kind: 'delta', text: 'Half an answer' });
    await waitFor(() => expect(answerText()).toBe('Half an answer'));
    await user.click(screen.getByRole('button', { name: 'Stop' }));
    expect(tauri.invoke).toHaveBeenCalledWith('cancel_answer', { answerId: 'answer-1' });
    step({ kind: 'stopped' });
    expect(await screen.findByTestId('answer-status')).toHaveTextContent(
      'Stopped. The answer so far is kept.',
    );
    expect(answerText()).toBe('Half an answer');
  });

  it('keeps a partial answer when the service fails, and Try again asks again', async () => {
    const user = await openAsk();
    await ask(user);
    step({ kind: 'delta', text: 'Partly [1]' });
    step({
      kind: 'error',
      code: 'network_error',
      message: 'x',
      partial: true,
      cited: [1],
      dropped: 0,
    });
    const alert = within(await screen.findByRole('alert'));
    expect(alert.getByText('The answer stopped early')).toBeInTheDocument();
    expect(
      alert.getByText('Atlas lost the connection to the chat service. Is it still running?'),
    ).toBeInTheDocument();
    expect(answerText()).toBe('Partly 1');
    await user.click(alert.getByRole('button', { name: 'Try again' }));
    expect(asks()).toHaveLength(2);
    expect(asks()[1][1]).toEqual({
      query: { question: 'how do harmonic drives work', sourceRootId: null, fileType: null },
    });
  });

  it.each([
    ['no_answer', 'The model finished without writing an answer.'],
    ['timeout', 'The chat service stopped answering for too long.'],
    ['authentication_failed', 'The chat service refused the key.'],
    ['service_error', 'The chat service reported a problem'],
  ] as const)('words a failure before any text: %s', async (code, words) => {
    const user = await openAsk();
    await ask(user);
    step({ kind: 'error', code, message: 'x', partial: false, cited: [], dropped: 0 });
    const alert = within(await screen.findByRole('alert'));
    expect(alert.getByText("Atlas couldn't get an answer")).toBeInTheDocument();
    expect(alert.getByText(new RegExp(words.replace(/[.]/g, '\\.')))).toBeInTheDocument();
    expect(screen.queryByRole('article', { name: 'Answer' })).toBeNull();
  });

  it('points to Settings when there is no chat model or external AI is off', async () => {
    const user = await openAsk();
    tauri.setHandler('ask_atlas', () => {
      throw tauri.apiError(
        'chat_model_needed',
        'Ask Atlas needs a chat model.',
        'Choose one in Settings.',
      );
    });
    await ask(user);
    const alert = within(await screen.findByRole('alert'));
    expect(alert.getByText('Ask Atlas needs a chat model')).toBeInTheDocument();
    await user.click(alert.getByRole('button', { name: 'Set up AI models' }));
    await h1('Settings');
  });

  it('keeps the answer while switching views, and renders bold and italic only', async () => {
    const user = await openAsk();
    await ask(user);
    step({ kind: 'delta', text: 'It is **Candide** by *Voltaire* <b>not bold</b> [1].' });
    step({ kind: 'done', cited: [1], dropped: 0 });
    const article = await screen.findByRole('article', { name: 'Answer' });
    expect(within(article).getByText('Candide').tagName).toBe('STRONG');
    expect(within(article).getByText('Voltaire').tagName).toBe('EM');
    expect(article.querySelector('b')).toBeNull();
    expect(article).toHaveTextContent('<b>not bold</b>');
    const nav = screen.getByRole('navigation', { name: 'Main' });
    await user.click(within(nav).getByRole('button', { name: 'Home' }));
    await user.click(within(nav).getByRole('button', { name: 'Search' }));
    expect(screen.getByRole('radio', { name: 'Ask Atlas' })).toBeChecked();
    expect(screen.getByRole('article', { name: 'Answer' })).toHaveTextContent(
      'It is Candide by Voltaire',
    );
  });

  it('counts characters near the 2,000 limit', async () => {
    const user = await openAsk();
    const field = screen.getByLabelText('Ask your library');
    await user.click(field);
    await user.paste('a'.repeat(1850));
    expect(screen.getByTestId('ask-counter')).toHaveTextContent('1,850 of 2,000 characters');
  });
});

describe('research trails', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const statusBar = () => within(screen.getByRole('contentinfo', { name: 'Status' }));
  const nav = () => within(screen.getByRole('navigation', { name: 'Main' }));
  const recordedOpens = () => tauri.invoke.mock.calls.filter(([c]) => c === 'record_open');

  it('shows the indicator, which pauses and resumes recording', async () => {
    const user = await launchOpen();
    const button = await statusBar().findByRole('button', { name: 'Trail: recording' });
    expect(button).toHaveAttribute('aria-pressed', 'true');
    await user.click(button);
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: expect.objectContaining({
          privacy: expect.objectContaining({ recordTrails: false }),
        }),
      }),
    );
    const paused = await statusBar().findByRole('button', { name: 'Trail: paused' });
    expect(paused).toHaveAttribute('aria-pressed', 'false');
    await user.click(paused);
    expect(await statusBar().findByRole('button', { name: 'Trail: recording' })).toBeVisible();
  });

  it('says once when a step could not be saved, and when trails are unavailable', async () => {
    tauri.setHandler('get_recording_status', () => tauri.recordingStatus({ state: 'unavailable' }));
    const user = await launchOpen();
    events.emit('trail-changed', tauri.recordingStatus({ failed: true }));
    expect(await statusBar().findByTestId('trail-failed')).toHaveTextContent(
      "Atlas couldn't save this step",
    );
    events.emit('trail-changed', tauri.recordingStatus({ currentTrailId: 't1', stepCount: 2 }));
    await waitFor(() => expect(statusBar().queryByTestId('trail-failed')).toBeNull());
    events.emit('trail-changed', tauri.recordingStatus({ state: 'unavailable' }));
    await user.click(await statusBar().findByRole('button', { name: 'Trails unavailable' }));
    await h1('Settings');
  });

  it('starts a new trail from beside the search mode', async () => {
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Search' }));
    await h1('Search');
    await user.click(screen.getByRole('button', { name: 'New trail' }));
    expect(tauri.invoke).toHaveBeenCalledWith('start_trail', undefined);
    expect(await screen.findByTestId('new-trail-status')).toHaveTextContent(
      'The next search starts a new trail.',
    );
    events.emit('trail-changed', tauri.recordingStatus({ currentTrailId: 't2', stepCount: 1 }));
    await waitFor(() => expect(screen.getByTestId('new-trail-status')).toHaveTextContent(''));
    // Paused, there is nothing to start.
    trailsStore.status = tauri.recordingStatus({ state: 'paused' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'New trail' })).toBeDisabled());
  });

  it("shows Home's note after the first step until it is dismissed", async () => {
    tauri.setHandler('get_recording_status', () => tauri.recordingStatus({ firstStepNote: true }));
    const user = await launchOpen();
    const note = await screen.findByTestId('trail-note');
    expect(note).toHaveTextContent(
      'Atlas now keeps a trail of your searches and questions on this computer, so you can pick them up later. Pause it or delete trails any time.',
    );
    await user.click(within(note).getByRole('button', { name: 'Got it' }));
    expect(tauri.invoke).toHaveBeenCalledWith('dismiss_trail_note', undefined);
    await waitFor(() => expect(screen.queryByTestId('trail-note')).toBeNull());
  });

  it('keeps the choices in Privacy, and deletes all trails after naming how many', async () => {
    tauri.setHandler('get_knowledge_status', () => tauri.knowledgeStatus({ trails: 12 }));
    tauri.setHandler('delete_all_trails', () => 12);
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Settings' }));
    const section = within(await screen.findByRole('region', { name: 'Research trails' }));
    expect(section.getByText(/stays on this computer/)).toBeInTheDocument();
    expect(section.getByRole('checkbox', { name: 'Record my trails' })).toBeChecked();

    await user.click(
      section.getByRole('checkbox', { name: 'Remove unpinned trails after 90 days' }),
    );
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: expect.objectContaining({
          privacy: expect.objectContaining({ expireTrails: false, recordTrails: true }),
        }),
      }),
    );
    await user.click(section.getByRole('checkbox', { name: 'Record my trails' }));
    expect(await section.findByText(/Paused: nothing is recorded/)).toBeInTheDocument();
    expect(await statusBar().findByRole('button', { name: 'Trail: paused' })).toBeVisible();

    // Keep leaves everything; Delete names the count first.
    await user.click(section.getByRole('button', { name: 'Delete all trails…' }));
    let confirm = within(
      await section.findByRole('group', { name: 'Confirm deleting all trails' }),
    );
    await user.click(confirm.getByRole('button', { name: 'Keep' }));
    expect(tauri.invoke).not.toHaveBeenCalledWith('delete_all_trails', undefined);
    await user.click(section.getByRole('button', { name: 'Delete all trails…' }));
    confirm = within(await section.findByRole('group', { name: 'Confirm deleting all trails' }));
    expect(
      confirm.getByText('Delete all 12 trails? Bookmarks and saved findings are kept.'),
    ).toBeInTheDocument();
    await user.click(confirm.getByRole('button', { name: 'Delete' }));
    expect(await section.findByTestId('trails-deleted')).toHaveTextContent('Deleted 12 trails.');
    expect(tauri.invoke).toHaveBeenCalledWith('delete_all_trails', undefined);
  });

  it('records only a submitted search, and opens from a result but never from the Library', async () => {
    const hit = tauri.searchHit({ fileInstanceId: 'h1' });
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({ hits: [hit], matchedDocuments: 1 }),
    );
    const intents = () =>
      tauri.invoke.mock.calls
        .filter(([c]) => c === 'search_documents')
        .map(([, a]) => (a as { intent: string }).intent);
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'gears{Enter}');
    await h1('Search');
    await waitFor(() => expect(intents()).toEqual(['submit']));
    await searchStore.goTo(1);
    await searchStore.setFilters({ fileType: 'pdf' });
    searchStore.refreshLive();
    await waitFor(() => expect(intents()).toEqual(['submit', 'refresh', 'refresh', 'refresh']), {
      timeout: 3000,
    });

    await user.click(await screen.findByRole('button', { name: `Details for ${hit.displayName}` }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'h1',
        via: { kind: 'result' },
        action: 'details',
        place: null,
      }),
    );
    const d = await screen.findByRole('dialog');
    await user.click(within(d).getByRole('button', { name: /^Open / }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'h1',
        via: { kind: 'result' },
        action: 'open',
        place: null,
      }),
    );
    await user.click(within(d).getByRole('button', { name: 'Close details' }));

    // The Library: details and Open, and no step.
    const before = recordedOpens().length;
    tauri.setHandler('list_documents', () =>
      tauri.documentPage({ items: [tauri.fileInstance({ id: 'f1' })], matchedSoFar: 1 }),
    );
    await user.click(nav().getByRole('button', { name: 'Library' }));
    await screen.findByRole('table');
    await user.click(screen.getAllByRole('button', { name: /^Details for / })[0]);
    const panel = await screen.findByRole('dialog');
    await user.click(within(panel).getByRole('button', { name: /^Open / }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('open_original', expect.anything()),
    );
    expect(recordedOpens()).toHaveLength(before);
  });

  it('records a citation opened from an answer, with the answer and its number', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('ask_atlas', () => tauri.askStarted());
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Search' }));
    await h1('Search');
    await user.click(screen.getByRole('radio', { name: 'Ask Atlas' }));
    await user.type(screen.getByLabelText('Ask your library'), 'how do harmonic drives work');
    await user.click(screen.getByRole('button', { name: 'Ask' }));
    const step = (s: import('./lib/api/bindings/AnswerStep').AnswerStep) =>
      events.emit('answer-stream', { answerId: 'answer-1', step: s });
    step({ kind: 'delta', text: 'High reduction [1].' });
    step({ kind: 'done', cited: [1], dropped: 0 });
    await user.click(
      await screen.findByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' }),
    );
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'file-1',
        via: { kind: 'citation', answerId: 'answer-1', n: 1 },
        action: 'details',
        place: null,
      }),
    );
  });
});

describe('the Trails screen', () => {
  beforeEach(resetAll);

  const nav = () => within(screen.getByRole('navigation', { name: 'Main' }));
  const calls = (command: string) =>
    tauri.invoke.mock.calls.filter(([c]) => c === command).map(([, a]) => a);

  async function openTrails(page = tauri.trailSummary()) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot({ id: 'root-1' })]);
    tauri.setHandler('list_trails', () => ({
      trails: [page],
      page: 0,
      pageSize: 25,
      total: 1,
    }));
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await h1('Trails');
    return user;
  }

  /** Opens the mock's trail: a search, a question, an open and a note. */
  async function openTimeline() {
    const user = await openTrails();
    await user.click(await screen.findByRole('button', { name: 'harmonic drives' }));
    await screen.findByRole('heading', { level: 2, name: 'harmonic drives' });
    return user;
  }

  it('says what trails are before there is one', async () => {
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await h1('Trails');
    expect(
      await screen.findByText(
        'Atlas keeps a trail of what you search for and ask, on this computer only. Your first search starts one.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText('Filter trails')).toBeNull();
  });

  it('lists trails with their dates, steps and pin, and pins one', async () => {
    const user = await openTrails(tauri.trailSummary({ stepCount: 4 }));
    const row = within(await screen.findByTestId('trail-row'));
    expect(row.getByText(/Nov 8, 2026 · .* · 4 steps/)).toBeInTheDocument();
    const pin = row.getByRole('button', { name: 'Pin harmonic drives' });
    expect(pin).toHaveAttribute('aria-pressed', 'false');
    await user.click(pin);
    expect(tauri.invoke).toHaveBeenCalledWith('pin_trail', { trailId: 'trail-1', pinned: true });
  });

  it('filters by titles, questions and searches, and pages', async () => {
    const user = await openTrails();
    tauri.setHandler('list_trails', (args) => {
      const q = args?.query as { filter: string; page: number };
      if (q.filter === 'gears') return { trails: [], page: 0, pageSize: 25, total: 0 };
      return {
        trails: Array.from({ length: 25 }, (_, i) =>
          tauri.trailSummary({ id: `t${i + q.page * 25}`, label: `trail ${i + q.page * 25}` }),
        ),
        page: q.page,
        pageSize: 25,
        total: 30,
      };
    });
    await user.type(screen.getByLabelText('Filter trails'), 'gears');
    expect(await screen.findByText('No trails match “gears”')).toBeInTheDocument();
    await user.clear(screen.getByLabelText('Filter trails'));
    expect(await screen.findByTestId('trail-range')).toHaveTextContent('Showing 1–25 of 30 trails');
    await user.click(screen.getByRole('button', { name: 'Next' }));
    await waitFor(() =>
      expect(calls('list_trails').at(-1)).toEqual({
        query: { filter: '', page: 1, pageSize: 25 },
      }),
    );
  });

  it('shows each step as recorded, with what became of its sources', async () => {
    await openTimeline();
    const steps = screen.getAllByTestId('trail-step');
    expect(steps.map((s) => s.dataset.kind)).toEqual(['search', 'ask', 'open', 'note']);

    const search = within(steps[0]);
    expect(search.getByRole('heading', { name: 'Searched for “harmonic drives”' })).toBeVisible();
    expect(search.getByText('in Books · PDFs only')).toBeInTheDocument();
    expect(search.getByText('14 documents matched then')).toBeInTheDocument();
    expect(search.getByRole('button', { name: 'Details for Harmonic Drives' })).toBeVisible();

    // The answer as recorded: bold and italic, never markup; not live.
    const ask = within(steps[1]);
    const answer = ask.getByRole('article', { name: 'Answer to “how do harmonic drives work”' });
    expect(answer.querySelector('strong')).toHaveTextContent('flexible');
    expect(answer.querySelector('em')).toHaveTextContent('high');
    expect(answer.querySelector('b')).toBeNull();
    expect(answer).toHaveTextContent('<b>not</b> markup');
    expect(answer).not.toHaveAttribute('aria-live');
    expect(ask.getByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' })).toBeVisible();
    expect(ask.getByTestId('weak-note')).toBeInTheDocument();
    expect(ask.getByTestId('dropped-note')).toHaveTextContent(
      "One reference the model gave didn't match any source and was removed.",
    );
    const then = within(ask.getByRole('region', { name: 'Sources then (3)' }));
    const sourcesThen = then.getAllByTestId('trail-source');
    expect(within(sourcesThen[1]).getByTestId('anchor-note')).toHaveTextContent(
      "This file isn't in your library any more.",
    );
    expect(within(sourcesThen[1]).getByText('Lost words.')).toBeVisible();
    expect(within(sourcesThen[1]).queryByRole('button', { name: /Details/ })).toBeNull();
    expect(within(sourcesThen[2]).getByTestId('anchor-note')).toHaveTextContent(
      "Atlas hasn't read this file yet.",
    );
    expect(ask.getByTestId('step-note')).toHaveTextContent('Check the 1957 patent.');

    const open = within(steps[2]);
    expect(open.getByRole('heading', { name: 'Opened Harmonic Drives' })).toBeVisible();
    expect(open.getByText('from a citation · details, then opened')).toBeInTheDocument();
    expect(open.getByTestId('anchor-note')).toHaveTextContent(
      'This passage has moved within the document since.',
    );
    expect(open.getByRole('button', { name: 'Open Harmonic Drives' })).toBeVisible();

    expect(within(steps[3]).getByText('Compare with cycloidal drives.')).toBeVisible();
  });

  it('opens a cited source with the passage that was sent, recorded as from a trail', async () => {
    const user = await openTimeline();
    const ask = within(screen.getAllByTestId('trail-step')[1]);
    await user.click(ask.getByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' }));
    const d = within(await screen.findByRole('dialog'));
    const cited = within(await d.findByTestId('cited-passage'));
    expect(cited.getByText(/flexible spline meshes/)).toBeInTheDocument();
    expect(cited.getByText('Sent to the model for this question on Nov 8, 2026.')).toBeVisible();
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'file-1',
        via: { kind: 'trail' },
        action: 'details',
        place: null,
      }),
    );
    await user.click(d.getByRole('button', { name: 'Close details' }));
    // A chip whose file has gone goes to what the trail kept of it.
    await user.click(ask.getByRole('button', { name: 'Source 2: Gone Book, Page 12' }));
    expect(document.activeElement).toHaveAttribute('id', 'step-2-source-2');
  });

  it('continues the trail, and searches and asks again from it', async () => {
    const user = await openTimeline();
    await user.click(screen.getByRole('button', { name: 'Continue this trail' }));
    expect(await screen.findByTestId('continuing')).toHaveTextContent('New steps join this trail.');
    expect(calls('continue_trail')).toEqual([{ trailId: 'trail-1' }]);

    await user.click(screen.getByRole('button', { name: 'Search again' }));
    await h1('Search');
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('search_documents', {
        query: expect.objectContaining({
          text: 'harmonic drives',
          sourceRootId: 'root-1',
          fileType: 'pdf',
          page: 0,
        }),
        intent: 'submit',
      }),
    );
    expect(screen.getByLabelText('Search your library')).toHaveValue('harmonic drives');

    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(await screen.findByRole('button', { name: 'Ask again' }));
    await h1('Search');
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('ask_atlas', {
        query: { question: 'how do harmonic drives work', sourceRootId: null, fileType: null },
      }),
    );
    expect(screen.getByRole('radio', { name: 'Ask Atlas' })).toBeChecked();
    expect(calls('continue_trail')).toHaveLength(3);
  });

  it('searches everything again when the recorded folder has gone, and says so', async () => {
    tauri.setHandler('list_source_roots', () => []);
    tauri.setHandler('list_trails', () => ({
      trails: [tauri.trailSummary()],
      page: 0,
      pageSize: 25,
      total: 1,
    }));
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(await screen.findByRole('button', { name: 'harmonic drives' }));
    await user.click(await screen.findByRole('button', { name: 'Search again' }));
    expect(await screen.findByTestId('search-notice')).toHaveTextContent(
      "The folder “Books” isn't in your library any more, so Atlas searched all of it.",
    );
    expect(calls('search_documents').at(-1)).toMatchObject({
      query: { sourceRootId: null, fileType: 'pdf' },
    });
  });

  it('renames, pins, notes and deletes from the timeline', async () => {
    const user = await openTimeline();
    await user.click(screen.getByRole('button', { name: 'Rename' }));
    const title = screen.getByLabelText('Trail title');
    expect(title).toHaveFocus();
    await user.type(title, 'Gears{Enter}');
    expect(tauri.invoke).toHaveBeenCalledWith('rename_trail', {
      trailId: 'trail-1',
      title: 'Gears',
    });
    await waitFor(() => expect(calls('get_trail')).toHaveLength(2));

    await user.click(screen.getByRole('button', { name: 'Pin' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pin_trail', { trailId: 'trail-1', pinned: true });

    // A note on a step, with the count near the limit, and a refusal shown.
    const first = within(screen.getAllByTestId('trail-step')[0]);
    await user.click(first.getByRole('button', { name: 'Add a note' }));
    const box = first.getByLabelText('Note on this step');
    await user.click(box);
    await user.paste('a'.repeat(3700));
    expect(first.getByText('300 characters left')).toBeInTheDocument();
    tauri.setHandler('add_trail_note', () => {
      throw { code: 'text_too_long', message: 'Keep the note to 4000 characters.' };
    });
    await user.click(first.getByRole('button', { name: 'Save note' }));
    expect(await screen.findByText('Keep the note to 4000 characters.')).toBeInTheDocument();
    tauri.setHandler('add_trail_note', () => null);
    await user.click(first.getByRole('button', { name: 'Save note' }));
    expect(calls('add_trail_note').at(-1)).toEqual({
      trailId: 'trail-1',
      stepId: 'step-1',
      text: 'a'.repeat(3700),
    });

    // A note of its own at the end.
    await user.click(screen.getByRole('button', { name: 'Add a note to this trail' }));
    await user.type(screen.getByLabelText('A note for this trail'), 'Next: the patent');
    await user.click(screen.getByRole('button', { name: 'Save note' }));
    expect(calls('add_trail_note').at(-1)).toEqual({
      trailId: 'trail-1',
      stepId: null,
      text: 'Next: the patent',
    });

    // A step, after confirming.
    await user.click(
      within(screen.getAllByTestId('trail-step')[0]).getByRole('button', {
        name: 'Delete this step',
      }),
    );
    const confirm = within(screen.getByRole('group', { name: 'Confirm deleting this step' }));
    await user.click(confirm.getByRole('button', { name: 'Delete' }));
    expect(tauri.invoke).toHaveBeenCalledWith('delete_trail_step', { stepId: 'step-1' });

    // The trail, after confirming: back to the list.
    await user.click(screen.getByRole('button', { name: 'Delete…' }));
    const sure = within(screen.getByRole('group', { name: 'Confirm deleting this trail' }));
    expect(
      sure.getByText('Delete this trail and its 4 steps? Bookmarks and saved findings are kept.'),
    ).toBeInTheDocument();
    await user.click(sure.getByRole('button', { name: 'Delete' }));
    expect(tauri.invoke).toHaveBeenCalledWith('delete_trail', { trailId: 'trail-1' });
    expect(await screen.findByTestId('trail-row')).toBeInTheDocument();
  });

  it('shows a new step when one joins the open trail', async () => {
    await openTimeline();
    expect(calls('get_trail')).toHaveLength(1);
    events.emit(
      'trail-changed',
      tauri.recordingStatus({ currentTrailId: 'trail-1', stepCount: 5 }),
    );
    await waitFor(() => expect(calls('get_trail')).toHaveLength(2));
    expect(screen.getByTestId('continuing')).toBeInTheDocument();
  });

  it('puts the three most recent trails on Home, each opening its timeline', async () => {
    tauri.setHandler('list_trails', () => ({
      trails: [
        tauri.trailSummary({ id: 'a', label: 'harmonic drives' }),
        tauri.trailSummary({ id: 'b', label: 'stoic letters' }),
        tauri.trailSummary({ id: 'c', label: 'sourdough' }),
      ],
      page: 0,
      pageSize: 3,
      total: 9,
    }));
    const user = await launchOpen();
    const section = within(
      await screen.findByRole('region', { name: 'Pick up where you left off' }),
    );
    expect(section.getAllByTestId('recent-trail')).toHaveLength(3);
    await user.click(section.getByRole('button', { name: 'stoic letters' }));
    await h1('Trails');
    expect(tauri.invoke).toHaveBeenCalledWith('get_trail', { trailId: 'b' });
  });

  it('leaves Home without the section when there are no trails', async () => {
    await launchOpen();
    await waitFor(() => expect(calls('list_trails').length).toBeGreaterThan(0));
    expect(screen.queryByRole('region', { name: 'Pick up where you left off' })).toBeNull();
  });
});

describe('bookmarks and saved findings', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const nav = () => within(screen.getByRole('navigation', { name: 'Main' }));
  const calls = (command: string) =>
    tauri.invoke.mock.calls.filter(([c]) => c === command).map(([, a]) => a);

  async function searchFor(text: string) {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({ hits: [tauri.searchHit({ fileInstanceId: 'h1' })], matchedDocuments: 1 }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), `${text}{Enter}`);
    await h1('Search');
    return user;
  }

  async function answered() {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('ask_atlas', () => tauri.askStarted());
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Search' }));
    await user.click(screen.getByRole('radio', { name: 'Ask Atlas' }));
    await user.type(screen.getByLabelText('Ask your library'), 'how do harmonic drives work');
    await user.click(screen.getByRole('button', { name: 'Ask' }));
    const step = (s: import('./lib/api/bindings/AnswerStep').AnswerStep) =>
      events.emit('answer-stream', { answerId: 'answer-1', step: s });
    return { user, step };
  }

  it('bookmarks a search result and says so', async () => {
    const hit = tauri.searchHit({ fileInstanceId: 'h1' });
    const user = await searchFor('gears');
    await user.click(await screen.findByRole('button', { name: `Bookmark: ${hit.displayName}` }));
    expect(calls('save_bookmark')).toEqual([
      { target: { kind: 'file', fileId: 'h1', place: { kind: 'result' } } },
    ]);
    expect(await screen.findByTestId('save-status-bookmark-hit-h1')).toHaveTextContent(
      'Bookmarked',
    );
  });

  it('bookmarks the document from its details, and lists its bookmarks there', async () => {
    tauri.setHandler('list_saved', (args) =>
      (args?.query as { documentId: string | null }).documentId === 'doc-1'
        ? tauri.savedPage({
            items: [{ item: 'bookmark', ...tauri.bookmark({ note: 'The key page.' }) }],
            total: 1,
          })
        : tauri.savedPage(),
    );
    const user = await searchFor('gears');
    await user.click(await screen.findByRole('button', { name: /^Details for / }));
    const d = within(await screen.findByRole('dialog'));
    const here = within(await d.findByTestId('bookmarks-here'));
    expect(here.getByRole('heading', { name: 'Bookmarks in this document' })).toBeVisible();
    expect(here.getByText('Page 12')).toBeVisible();
    expect(here.getByText('The key page.')).toBeVisible();
    await user.click(d.getByRole('button', { name: 'Bookmark: Harmonic Drives' }));
    expect(calls('save_bookmark').at(-1)).toEqual({
      target: { kind: 'file', fileId: 'h1', place: { kind: 'document' } },
    });
  });

  it('keeps an answer once it is done, and its cited passage and sources', async () => {
    const { user, step } = await answered();
    step({ kind: 'delta', text: 'High reduction [1].' });
    expect(screen.queryByRole('button', { name: /^Save as a finding/ })).toBeNull();
    step({ kind: 'done', cited: [1], dropped: 0 });
    await user.click(
      await screen.findByRole('button', {
        name: 'Save as a finding: how do harmonic drives work',
      }),
    );
    expect(calls('save_finding').at(-1)).toEqual({
      source: { kind: 'answer', answerId: 'answer-1' },
    });
    expect(await screen.findByTestId('save-status-finding-answer-1')).toHaveTextContent(
      'Saved as a finding',
    );

    // A source in Sources used.
    await user.click(
      screen.getByRole('button', { name: 'Bookmark: Source 2: Strain Wave Gearing' }),
    );
    expect(calls('save_bookmark').at(-1)).toEqual({
      target: {
        kind: 'file',
        fileId: 'file-2',
        place: { kind: 'citation', answerId: 'answer-1', n: 2 },
      },
    });

    // The cited passage, from its chip: as a bookmark and as a finding.
    await user.click(screen.getByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' }));
    const cited = within(await screen.findByTestId('cited-passage'));
    const citation = {
      kind: 'file',
      fileId: 'file-1',
      place: { kind: 'citation', answerId: 'answer-1', n: 1 },
    };
    await user.click(cited.getByRole('button', { name: 'Bookmark this passage' }));
    expect(calls('save_bookmark').at(-1)).toEqual({ target: citation });
    await user.click(cited.getByRole('button', { name: 'Save as a finding' }));
    expect(calls('save_finding').at(-1)).toEqual({
      source: { kind: 'passage', target: citation },
    });
  });

  it('says why something could not be kept', async () => {
    tauri.setHandler('save_bookmark', () => {
      throw { code: 'save_source_gone', message: "Atlas can't find that any more." };
    });
    const user = await searchFor('gears');
    await user.click(await screen.findByRole('button', { name: /^Bookmark: / }));
    expect(await screen.findByTestId('save-status-bookmark-hit-h1')).toHaveTextContent(
      "Atlas can't find that any more.",
    );
  });

  it('keeps a trail’s question and bookmarks a recorded source', async () => {
    tauri.setHandler('list_trails', () => ({
      trails: [tauri.trailSummary()],
      page: 0,
      pageSize: 25,
      total: 1,
    }));
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(await screen.findByRole('button', { name: 'harmonic drives' }));
    const steps = await screen.findAllByTestId('trail-step');
    const ask = within(steps[1]);
    await user.click(
      ask.getByRole('button', { name: 'Save as a finding: how do harmonic drives work' }),
    );
    expect(calls('save_finding').at(-1)).toEqual({ source: { kind: 'step', stepId: 'step-2' } });
    const result = within(steps[0]);
    await user.click(result.getByRole('button', { name: 'Bookmark: Harmonic Drives' }));
    expect(calls('save_bookmark').at(-1)).toEqual({
      target: { kind: 'step', stepId: 'step-1', index: 0 },
    });
    // Opening a recorded source passes what its panel's bookmark keeps.
    await user.click(ask.getByRole('button', { name: 'Source 1: Harmonic Drives, Page 12' }));
    const cited = within(await screen.findByTestId('cited-passage'));
    await user.click(cited.getByRole('button', { name: 'Bookmark this passage' }));
    expect(calls('save_bookmark').at(-1)).toEqual({
      target: { kind: 'step', stepId: 'step-2', index: 0 },
    });
  });

  it('shows the Saved tab beside Trails, empty at first', async () => {
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    const tabs = within(screen.getByRole('group', { name: 'Show' }));
    expect(tabs.getAllByRole('radio').map((r) => r.closest('label')?.textContent?.trim())).toEqual([
      'Trails',
      'Saved',
    ]);
    await user.click(tabs.getByRole('radio', { name: 'Saved' }));
    expect(
      await screen.findByText(
        'Bookmarks and findings you save show here. Bookmark a passage or save an answer to keep it.',
      ),
    ).toBeInTheDocument();
  });

  it('lists what was saved, filters it, edits notes and deletes', async () => {
    tauri.setHandler('list_saved', (args) => {
      const q = args?.query as { kind: string | null; filter: string };
      if (q.filter === 'nothing') return tauri.savedPage();
      const items = [
        { item: 'finding' as const, ...tauri.finding() },
        {
          item: 'bookmark' as const,
          ...tauri.bookmark({
            resolved: tauri.resolved({ state: 'no_file', fileInstanceId: null }),
          }),
        },
        {
          item: 'bookmark' as const,
          ...tauri.bookmark({ id: 'bm-2', anchor: tauri.anchor({ title: 'Gears' }) }),
        },
      ].filter((i) => !q.kind || i.item === q.kind);
      return tauri.savedPage({ items, total: items.length });
    });
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(screen.getByRole('radio', { name: 'Saved' }));
    const items = await screen.findAllByTestId('saved-item');
    expect(items.map((i) => i.dataset.kind)).toEqual(['finding', 'bookmark', 'bookmark']);

    // A finding: its answer as kept, its sources, its note.
    const f = within(items[0]);
    expect(
      f.getByRole('heading', { name: 'Answer to “how do harmonic drives work”' }),
    ).toBeVisible();
    expect(f.getByRole('article').querySelector('strong')).toHaveTextContent('high');
    expect(f.getByTestId('saved-note')).toHaveTextContent('For chapter 2.');

    // A bookmark whose file has gone says so; one still there opens, as
    // reached from a bookmark.
    expect(within(items[1]).getByTestId('anchor-note')).toHaveTextContent(
      "This file isn't in your library any more.",
    );
    await user.click(within(items[2]).getByRole('button', { name: 'Details for Gears' }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'file-1',
        via: { kind: 'bookmark' },
        action: 'details',
        place: null,
      }),
    );
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Close details' }),
    );

    // Filters.
    await user.click(screen.getByRole('radio', { name: 'Findings' }));
    await waitFor(() =>
      expect(calls('list_saved').at(-1)).toMatchObject({ query: { kind: 'finding' } }),
    );
    expect(await screen.findAllByTestId('saved-item')).toHaveLength(1);
    await user.click(screen.getByRole('radio', { name: 'All' }));
    await user.type(screen.getByLabelText('Filter saved'), 'nothing');
    expect(await screen.findByText('Nothing saved matches “nothing”')).toBeInTheDocument();
    await user.clear(screen.getByLabelText('Filter saved'));

    // A note, near the limit and refused over it, then saved.
    const bookmarkItem = within((await screen.findAllByTestId('saved-item'))[2]);
    await user.click(bookmarkItem.getByRole('button', { name: 'Add a note' }));
    const box = bookmarkItem.getByLabelText('Note');
    await user.click(box);
    await user.paste('b'.repeat(3800));
    expect(bookmarkItem.getByText('200 characters left')).toBeInTheDocument();
    tauri.setHandler('update_saved_note', () => {
      throw { code: 'text_too_long', message: 'Keep the note to 4000 characters.' };
    });
    await user.click(bookmarkItem.getByRole('button', { name: 'Save note' }));
    expect(await screen.findByText('Keep the note to 4000 characters.')).toBeInTheDocument();
    tauri.setHandler('update_saved_note', () => ({ item: 'bookmark', ...tauri.bookmark() }));
    await user.click(bookmarkItem.getByRole('button', { name: 'Save note' }));
    expect(calls('update_saved_note').at(-1)).toEqual({
      kind: 'bookmark',
      id: 'bm-2',
      note: 'b'.repeat(3800),
    });

    // Delete, after confirming.
    const first = within((await screen.findAllByTestId('saved-item'))[0]);
    await user.click(first.getByRole('button', { name: 'Delete…' }));
    const confirm = within(first.getByRole('group', { name: 'Confirm deleting' }));
    expect(
      confirm.getByText('Delete this finding? Its trail, if it still has one, is kept.'),
    ).toBeInTheDocument();
    await user.click(confirm.getByRole('button', { name: 'Delete' }));
    expect(calls('delete_saved').at(-1)).toEqual({ kind: 'finding', id: 'f-1' });
  });
});
