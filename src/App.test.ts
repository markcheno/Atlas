import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as events from './test/eventMock';
import * as tauri from './test/tauriMock';

vi.mock('@tauri-apps/api/core', () => import('./test/tauriMock'));
vi.mock('@tauri-apps/api/event', () => import('./test/eventMock'));

const { library: libraryStore } = await import('./lib/stores/library.svelte');
const { providers: providersStore } = await import('./lib/stores/providers.svelte');

const { default: App } = await import('./App.svelte');
const { view } = await import('./lib/stores/view.svelte');
const { theme } = await import('./lib/stores/theme.svelte');
const { workspace } = await import('./lib/stores/workspace.svelte');
const { sources } = await import('./lib/stores/sources.svelte');
const { inventory } = await import('./lib/stores/inventory.svelte');

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
    await user.keyboard('{Control>}3{/Control}');
    expect(await h1('Settings')).toHaveFocus();
    await user.keyboard('{Control>}2{/Control}');
    expect(await h1('Library')).toHaveFocus();
  });

  it('theme choice is applied and saved to the library settings', async () => {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(await screen.findByRole('radio', { name: 'Dark' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('update_settings', {
        settings: { appearance: { theme: 'dark' } },
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
    expect(screen.getByText('Only in the cloud')).toBeInTheDocument();
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

  it('Home search goes to the Library with the query', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search file names and folders'), 'meteor{Enter}');
    await screen.findByRole('heading', { level: 1, name: 'Library' });
    await waitFor(() => expect(lastQuery().text).toBe('meteor'));
  });
});

describe('AI models settings', () => {
  beforeEach(() => {
    resetAll();
    providersStore.clear();
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
    expect(await within(aiSection()).findByText('Ollama')).toBeInTheDocument();
    expect(within(aiSection()).getByText(/This computer/)).toBeInTheDocument();
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

  it('shows models and never claims chat or embedding support', async () => {
    const user = await openAiSettings();
    await user.click(within(aiSection()).getByRole('button', { name: 'Add a service…' }));
    const form = within(aiSection()).getByRole('form', { name: 'Add an AI service' });
    await user.click(within(form).getByRole('button', { name: 'Test' }));
    expect(await within(aiSection()).findByText('qwen3.5:9b')).toBeInTheDocument();
    expect(
      within(aiSection()).getByText(/Atlas will check what each model can do/),
    ).toBeInTheDocument();
    expect(aiSection().textContent).not.toMatch(/supports (chat|embeddings)/i);
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
        onThisComputer: true,
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
