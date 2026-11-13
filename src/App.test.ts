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
          background: { checkFolders: 'sixHours' },
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
    expect(within(list).getByText('● Available')).toBeInTheDocument();
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
    expect(screen.getByText('○ Not connected')).toBeInTheDocument();
    expect(screen.getByText('⚠ Atlas needs permission')).toBeInTheDocument();
    expect(screen.getByText('↳ Inside “Library”')).toBeInTheDocument();
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
    // Slice 4.1.3: a busy computer was not mentioned at all here.
    [
      { state: 'running', throttleReason: 'system_busy' },
      'Cataloged 1,204 files · Scanning Books · Going slowly · The computer is busy',
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
            activeState: null,
            lastScan: null,
          },
        ],
      }),
    );
    expect(status()).toHaveTextContent('Up to date · 1,204 files');
    expect(screen.getByTestId('root-line')).toHaveTextContent(
      '1,204 files · Checked 5 min ago · 12 only in the cloud · 3 missing',
    );
  });

  it('Pause, Resume, and Cancel call the right commands', async () => {
    const user = await launchWith(tauri.overview({ active: tauri.inventoryStatus() }));
    const bar = within(screen.getByRole('contentinfo', { name: 'Status' }));
    await user.click(bar.getByRole('button', { name: 'Pause cataloging' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pause_job', { jobId: 'job-1' });

    events.emit(
      'inventory-status',
      tauri.overview({ active: tauri.inventoryStatus({ state: 'paused_user' }) }),
    );
    await user.click(await bar.findByRole('button', { name: 'Resume cataloging' }));
    expect(tauri.invoke).toHaveBeenCalledWith('resume_job', { jobId: 'job-1' });

    await user.click(bar.getByRole('button', { name: 'Cancel cataloging' }));
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
      { state: 'running', throttleReason: 'power_unknown' },
      false,
      'Reading text · 3 of 10 files · Going slowly · Can’t tell if it’s plugged in',
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

  it('says when the document reader can’t start (Slice 4.1)', async () => {
    const root = (failedBecause: string) => ({
      sourceRootId: 'root-books',
      counts,
      lastState: 'failed_terminal' as const,
      activeJobId: null,
      activeState: null,
      failedBecause,
    });
    await launchWith(tauri.extractionOverview({ counts, roots: [root('worker_protocol')] }));
    expect(line()).toHaveTextContent(
      "Atlas couldn't start its document reader · Reinstall Atlas to fix it",
    );
    events.emit(
      'extraction-status',
      tauri.extractionOverview({ counts, roots: [root('path_undecodable')] }),
    );
    await waitFor(() => expect(line()).toHaveTextContent('Text waiting · 3 of 10 files read'));
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
            activeState: 'paused_user',
            failedBecause: null,
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
    expect(bar.getByRole('button', { name: 'Pause cataloging' })).toBeInTheDocument();
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
    expect(section.getByText('The log')).toBeInTheDocument();
    expect(section.getByTestId('log-bytes')).toHaveTextContent('48.0 KB');
    expect(section.getByText(/the log, or your trails/)).toBeInTheDocument();
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
    // Slice 4.1.3: "Going slowly" says why.
    [
      { state: 'running', throttleReason: 'system_busy' },
      {},
      'Indexing meaning · 3 of 10 files · Going slowly · The computer is busy',
    ],
    [
      { state: 'running', throttleReason: 'on_battery' },
      { rebuilding: true },
      'Rebuilding meaning index · 3 of 10 files · Slowed on battery · search uses the previous index',
    ],
    [{ state: 'running' }, { reading: true }, 'Indexing meaning · 3 files so far'],
    [
      { state: 'running' },
      { rebuilding: true },
      'Rebuilding meaning index · 3 of 10 files · search uses the previous index',
    ],
    [
      { state: 'paused_policy', pauseReason: 'on_battery' },
      {},
      'Paused indexing meaning on battery · Resumes when plugged in',
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
        roots: [
          {
            sourceRootId: 'root-books',
            counts,
            activeJobId: null,
            activeState: null,
            failedBecause: null,
          },
        ],
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

  // Slice 4.1.1 (found on the owner's library): a job that stopped for
  // good isn't active, and the line said only "Meaning waiting".
  const stopped = (failedBecause: string) =>
    withModel({
      roots: [
        { sourceRootId: 'root-books', counts, activeJobId: null, activeState: null, failedBecause },
      ],
    });

  it('says why indexing stopped for good, and sends the owner to Settings', async () => {
    const user = await launchWith(stopped('model_not_found'));
    expect(line()).toHaveTextContent(
      "Can't index meaning · The AI service no longer has the model · 3 of 10 files indexed",
    );
    expect(screen.getByTestId('root-meaning-line')).toHaveTextContent(
      "Can't index meaning · The AI service no longer has the model",
    );
    expect(screen.queryByRole('button', { name: 'Index meaning now' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Open Settings' }));
    await h1('Settings');
  });

  it('offers Index meaning now when the stop needs no setting changed', async () => {
    const user = await launchWith(stopped('index_unavailable'));
    expect(line()).toHaveTextContent(
      "Can't index meaning · Atlas couldn't open its meaning index · 3 of 10 files indexed",
    );
    await user.click(screen.getByRole('button', { name: 'Index meaning now' }));
    expect(tauri.invoke).toHaveBeenCalledWith('start_embedding', { sourceRootId: 'root-books' });
  });

  it('still says waiting when nothing stopped, and nothing once a stop has no file left', async () => {
    await launchWith(
      withModel({
        roots: [
          {
            sourceRootId: 'root-books',
            counts,
            activeJobId: null,
            activeState: null,
            failedBecause: null,
          },
        ],
      }),
    );
    expect(line()).toHaveTextContent('Meaning waiting · 3 of 10 files indexed');
    const done = tauri.embeddingCounts({ indexed: 10, total: 10 });
    events.emit(
      'embedding-status',
      withModel({
        counts: done,
        roots: [
          {
            sourceRootId: 'root-books',
            counts: done,
            activeJobId: null,
            activeState: null,
            failedBecause: 'model_not_found',
          },
        ],
      }),
    );
    await waitFor(() => expect(line()).toHaveTextContent('Meaning up to date · 10 files'));
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
          background: { checkFolders: 'sixHours' },
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
          background: { checkFolders: 'sixHours' },
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
          background: { checkFolders: 'sixHours' },
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
          background: { checkFolders: 'sixHours' },
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

  it('resolves an open trail again when Trails is shown again, after a rescan', async () => {
    const user = await openTimeline();
    const source = () =>
      within(
        within(screen.getAllByTestId('trail-step')[1])
          .getByRole('region', { name: 'Sources then (3)' })
          .querySelectorAll<HTMLElement>('[data-testid="trail-source"]')[1],
      );
    expect(source().getByTestId('anchor-note')).toHaveTextContent(
      "This file isn't in your library any more.",
    );
    // A rescan in Settings found the moved file; back in Trails it opens.
    await user.click(nav().getByRole('button', { name: 'Settings' }));
    const detail = tauri.trailDetail();
    const ask = detail.steps[1];
    ask.anchors[1] = {
      ...ask.anchors[1],
      state: 'found',
      documentId: ask.anchors[1].documentId ?? 'doc-9',
      fileInstanceId: 'file-moved',
      excerptFound: true,
    };
    tauri.setHandler('get_trail', () => detail);
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await waitFor(() => expect(source().queryByTestId('anchor-note')).toBeNull());
    expect(source().getByRole('button', { name: /^Details for / })).toBeVisible();
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
      'Interests and projects',
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

describe('interests, projects and suggestions', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const nav = () => within(screen.getByRole('navigation', { name: 'Main' }));
  const calls = (command: string) =>
    tauri.invoke.mock.calls.filter(([c]) => c === command).map(([, a]) => a);

  async function openTab() {
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(screen.getByRole('radio', { name: 'Interests and projects' }));
    return user;
  }

  it('says what interests and projects are before there is one, and creates one', async () => {
    const user = await openTab();
    expect(
      await screen.findByText(
        'Group trails, bookmarks, findings and documents under an interest or a project. Nothing needs one.',
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'New interest…' }));
    expect(screen.queryByLabelText('Goal (optional)')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await user.click(screen.getByRole('button', { name: 'New project…' }));
    await user.type(screen.getByLabelText('Name of the new project'), 'Thesis');
    await user.type(screen.getByLabelText('Goal (optional)'), 'A chapter on drives');
    await user.click(screen.getByRole('button', { name: 'Create' }));
    expect(calls('save_collection').at(-1)).toEqual({
      input: {
        id: null,
        kind: 'project',
        name: 'Thesis',
        description: null,
        goal: 'A chapter on drives',
        status: null,
      },
    });
    expect(tauri.invoke).toHaveBeenCalledWith('get_collection', { id: 'c-new' });
  });

  it('lists active, finished and archived ones, each opening its page', async () => {
    tauri.setHandler('list_collections', () => [
      tauri.collection(),
      tauri.collection({ id: 'c-2', kind: 'project', name: 'Thesis', status: 'finished' }),
      tauri.collection({ id: 'c-3', name: 'Old', status: 'archived' }),
    ]);
    const user = await openTab();
    for (const name of ['Active', 'Finished', 'Archived']) {
      expect(await screen.findByRole('heading', { name })).toBeVisible();
    }
    expect(screen.getAllByTestId('collection-row')).toHaveLength(3);
    expect(screen.getAllByTestId('collection-row')[0]).toHaveTextContent(
      'Interest · 1 trail · 1 finding · 1 bookmark · 1 document',
    );
    await user.click(screen.getByRole('button', { name: 'Mechanisms' }));
    expect(await screen.findByRole('heading', { level: 2, name: 'Mechanisms' })).toBeVisible();
  });

  it('shows a collection’s page, takes items out, and changes or deletes it', async () => {
    tauri.setHandler('list_collections', () => [tauri.collection()]);
    const user = await openTab();
    await user.click(await screen.findByRole('button', { name: 'Mechanisms' }));
    await screen.findByRole('heading', { level: 2, name: 'Mechanisms' });
    expect(screen.getByText('Gears and drives')).toBeVisible();
    expect(screen.getAllByTestId('collection-trail')).toHaveLength(1);
    expect(screen.getAllByTestId('collection-finding')).toHaveLength(1);
    expect(screen.getAllByTestId('collection-bookmark')).toHaveLength(1);
    const docs = screen.getAllByTestId('collection-document');
    expect(
      within(docs[0]).getByRole('button', { name: 'Details for Strain Wave Gearing' }),
    ).toBeVisible();
    expect(docs[1]).toHaveTextContent("Atlas hasn't read this document yet.");

    // Take a document out: only the link goes.
    await user.click(within(docs[0]).getByRole('button', { name: 'Remove from this interest' }));
    expect(calls('remove_from_collection').at(-1)).toEqual({
      collectionId: 'c-1',
      item: { kind: 'document', fileId: null, documentId: 'doc-2' },
    });

    // An interest has no Mark as finished; it can be archived and edited.
    expect(screen.queryByRole('button', { name: 'Mark as finished' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Archive' }));
    expect(calls('save_collection').at(-1)).toMatchObject({
      input: { id: 'c-1', status: 'archived' },
    });
    await user.click(screen.getByRole('button', { name: 'Edit' }));
    const field = screen.getByLabelText('Name');
    await user.clear(field);
    await user.type(field, 'Machines');
    expect(screen.queryByLabelText('Goal')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(calls('save_collection').at(-1)).toMatchObject({
      input: { id: 'c-1', name: 'Machines', goal: null },
    });

    // Delete, after a confirmation that keeps what it held.
    await user.click(screen.getByRole('button', { name: 'Delete…' }));
    const confirm = within(screen.getByRole('group', { name: 'Confirm deleting this collection' }));
    expect(
      confirm.getByText(
        'Delete the interest “Mechanisms”? Its trails, bookmarks and findings are kept.',
      ),
    ).toBeInTheDocument();
    await user.click(confirm.getByRole('button', { name: 'Delete' }));
    expect(calls('delete_collection').at(-1)).toEqual({ id: 'c-1' });
    expect(await screen.findByRole('button', { name: 'New interest…' })).toBeVisible();
  });

  it('marks a project as finished', async () => {
    tauri.setHandler('list_collections', () => [
      tauri.collection({ kind: 'project', name: 'Thesis', goal: 'A chapter' }),
    ]);
    tauri.setHandler('get_collection', () =>
      tauri.collectionDetail({
        collection: tauri.collection({ kind: 'project', name: 'Thesis', goal: 'A chapter' }),
      }),
    );
    const user = await openTab();
    await user.click(await screen.findByRole('button', { name: 'Thesis' }));
    expect(await screen.findByText('A chapter')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Mark as finished' }));
    expect(calls('save_collection').at(-1)).toMatchObject({
      input: { status: 'finished', goal: 'A chapter' },
    });
  });

  it('adds a trail, a saved item and a document to interests from Add to…', async () => {
    tauri.setHandler('list_collections', () => [
      tauri.collection(),
      tauri.collection({ id: 'c-2', kind: 'project', name: 'Thesis' }),
      tauri.collection({ id: 'c-3', name: 'Old', status: 'archived' }),
    ]);
    tauri.setHandler('get_collection_membership', (args) =>
      (args?.item as { kind: string }).kind === 'trail'
        ? { collectionIds: ['c-1'] }
        : { collectionIds: [] },
    );
    tauri.setHandler('list_trails', () => ({
      trails: [tauri.trailSummary()],
      page: 0,
      pageSize: 25,
      total: 1,
    }));
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(await screen.findByRole('button', { name: 'harmonic drives' }));
    await user.click(await screen.findByRole('button', { name: 'Add to… harmonic drives' }));
    const menu = within(
      await screen.findByRole('group', { name: 'Add harmonic drives to an interest or project' }),
    );
    // Active ones only, each saying whether the trail is in it.
    expect(menu.getByRole('button', { name: /Mechanisms/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(menu.getByRole('button', { name: /Thesis/ })).toHaveAttribute('aria-pressed', 'false');
    expect(menu.queryByRole('button', { name: /Old/ })).toBeNull();
    await user.click(menu.getByRole('button', { name: /Thesis/ }));
    expect(calls('add_to_collection').at(-1)).toEqual({
      collectionId: 'c-2',
      item: { kind: 'trail', id: 'trail-1' },
    });
    await user.click(menu.getByRole('button', { name: /Mechanisms/ }));
    expect(calls('remove_from_collection').at(-1)).toEqual({
      collectionId: 'c-1',
      item: { kind: 'trail', id: 'trail-1' },
    });
    // A new interest, created and added at once.
    await user.click(menu.getByRole('button', { name: 'New interest…' }));
    await user.type(menu.getByLabelText('Name of the new interest'), 'Gears');
    await user.click(menu.getByRole('button', { name: 'Create and add' }));
    await waitFor(() =>
      expect(calls('add_to_collection').at(-1)).toEqual({
        collectionId: 'c-new',
        item: { kind: 'trail', id: 'trail-1' },
      }),
    );

    // A saved bookmark.
    tauri.setHandler('list_saved', () =>
      tauri.savedPage({ items: [{ item: 'bookmark', ...tauri.bookmark() }], total: 1 }),
    );
    await user.click(screen.getByRole('radio', { name: 'Saved' }));
    await user.click(await screen.findByRole('button', { name: 'Add to… Harmonic Drives' }));
    await user.click(
      within(await screen.findByRole('group', { name: /^Add Harmonic Drives to/ })).getByRole(
        'button',
        { name: /Mechanisms/ },
      ),
    );
    expect(calls('add_to_collection').at(-1)).toEqual({
      collectionId: 'c-1',
      item: { kind: 'bookmark', id: 'bm-1' },
    });

    // A document, from its details.
    await user.click(
      within(screen.getByTestId('saved-item')).getByRole('button', { name: /^Details for/ }),
    );
    const d = within(await screen.findByRole('dialog'));
    await user.click(await d.findByRole('button', { name: 'Add to… Harmonic Drives' }));
    await user.click(
      within(await d.findByRole('group', { name: /^Add Harmonic Drives to/ })).getByRole('button', {
        name: /Thesis/,
      }),
    );
    expect(calls('add_to_collection').at(-1)).toEqual({
      collectionId: 'c-2',
      item: { kind: 'document', fileId: 'file-1', documentId: null },
    });
  });

  it('puts active interests and projects and suggestions on Home', async () => {
    tauri.setHandler('list_collections', () =>
      Array.from({ length: 8 }, (_, i) =>
        tauri.collection({
          id: `c-${i}`,
          name: `Interest ${i}`,
          status: i === 7 ? 'archived' : 'active',
        }),
      ),
    );
    tauri.setHandler('get_suggestions', () => [
      tauri.suggestion(),
      tauri.suggestion({
        hit: tauri.searchHit({
          fileInstanceId: 'file-8',
          documentId: 'doc-8',
          title: 'Gear Trains',
        }),
        reason: { seed: 'recent' },
        seedId: '',
      }),
    ]);
    const user = await launchOpen();
    const mine = within(await screen.findByRole('region', { name: 'Interests and projects' }));
    expect(mine.getAllByTestId('home-collection')).toHaveLength(6);
    const from = within(screen.getByRole('region', { name: 'From your library' }));
    const items = from.getAllByTestId('suggestion');
    expect(items[0]).toHaveTextContent('Related to your interest ‘Mechanisms’');
    expect(items[1]).toHaveTextContent('Related to your recent trails');
    await user.click(from.getByRole('button', { name: 'Details for Cycloidal Drives' }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'file-7',
        via: { kind: 'suggestion' },
        action: 'details',
        place: null,
      }),
    );
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Close details' }),
    );
    await user.click(from.getByRole('button', { name: 'Not for this: Gear Trains' }));
    expect(calls('dismiss_suggestion').at(-1)).toEqual({ seedId: '', documentId: 'doc-8' });
    // All of them, in the Trails view's tab.
    await user.click(mine.getByRole('button', { name: 'All interests and projects' }));
    await h1('Trails');
    expect(screen.getByRole('radio', { name: 'Interests and projects' })).toBeChecked();
  });

  it('leaves Home without either section when there is nothing to show', async () => {
    await launchOpen();
    await waitFor(() => expect(calls('get_suggestions').length).toBeGreaterThan(0));
    expect(screen.queryByRole('region', { name: 'Interests and projects' })).toBeNull();
    expect(screen.queryByRole('region', { name: 'From your library' })).toBeNull();
  });
});

describe('the page view', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const nav = () => within(screen.getByRole('navigation', { name: 'Main' }));
  const calls = (command: string) =>
    tauri.invoke.mock.calls.filter(([c]) => c === command).map(([, a]) => a);
  const pageDialog = () =>
    within(screen.getByLabelText('Close the page').closest('[role="dialog"]') as HTMLElement);

  async function citedPassage() {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('ask_atlas', () => tauri.askStarted());
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Search' }));
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
    const cited = within(await screen.findByTestId('cited-passage'));
    return { user, cited };
  }

  it('shows a cited page with its passage outlined and its text below', async () => {
    const { user, cited } = await citedPassage();
    const show = cited.getByRole('button', { name: 'Show the page' });
    await user.click(show);
    expect(calls('render_page').at(-1)).toEqual({
      fileId: 'file-1',
      pageNo: 12,
      widthPx: 1200,
      find: expect.stringContaining('flexible spline'),
      allowDownload: false,
    });
    const view = pageDialog();
    const figure = await view.findByTestId('page-image');
    expect(within(figure).getByRole('img', { name: 'Page 12 of 340' })).toHaveAttribute(
      'src',
      'data:image/png;base64,iVBORw0KGgo=',
    );
    // Drawn as SVG in the image's pixels, never with inline styles, which
    // the app's CSP blocks (Slice 3.9).
    const outlines = view.getAllByTestId('page-outline');
    expect(outlines).toHaveLength(1);
    expect(outlines[0].closest('svg')).toHaveAttribute('viewBox', '0 0 1200 1553');
    expect(['x', 'y', 'width', 'height'].map((k) => outlines[0].getAttribute(k))).toEqual([
      '139',
      '118',
      '604',
      '22',
    ]);
    expect(figure.querySelector('[style]')).toBeNull();
    expect(view.getByRole('heading', { name: "The page's text, as Atlas read it" })).toBeVisible();
    expect(view.getByText(/rigid circular spline/)).toBeVisible();
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'file-1',
        via: { kind: 'citation', answerId: 'answer-1', n: 1 },
        action: 'page',
        place: { pageNo: 12 },
      }),
    );

    // Another page: no passage looked for there.
    await user.click(view.getByRole('button', { name: 'Next page' }));
    await waitFor(() =>
      expect(calls('render_page').at(-1)).toMatchObject({ pageNo: 13, find: null }),
    );
    expect(await view.findByRole('img', { name: 'Page 13 of 340' })).toBeVisible();
    expect(view.queryByTestId('page-outline')).toBeNull();

    // The page field, bounded by the document.
    const field = view.getByLabelText('Page number');
    await user.clear(field);
    await user.type(field, '1{Enter}');
    await waitFor(() => expect(calls('render_page').at(-1)).toMatchObject({ pageNo: 1 }));
    expect(await view.findByRole('button', { name: 'Previous page' })).toBeDisabled();
    await user.clear(field);
    await user.type(field, '9999{Enter}');
    await waitFor(() => expect(calls('render_page').at(-1)).toMatchObject({ pageNo: 340 }));

    // Fit width, actual size; bookmark the page.
    await user.click(view.getByRole('radio', { name: 'Actual size' }));
    expect(view.getByTestId('page-image')).toHaveClass('actual');
    await user.click(view.getByRole('button', { name: 'Bookmark this page' }));
    expect(calls('save_bookmark').at(-1)).toEqual({
      target: { kind: 'file', fileId: 'file-1', place: { kind: 'page', pageNo: 340 } },
    });

    // Escape closes and focus returns to where it was opened.
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByLabelText('Close the page')).toBeNull());
    expect(show).toHaveFocus();
  });

  it('says when the passage is not on the page, and when the page cannot be shown', async () => {
    tauri.setHandler('render_page', () => tauri.pageImage({ found: false, rects: [] }));
    const { user, cited } = await citedPassage();
    await user.click(cited.getByRole('button', { name: 'Show the page' }));
    expect(
      await pageDialog().findByText(/The passage isn't marked: Atlas didn't find it on this page./),
    ).toBeVisible();
    await user.click(pageDialog().getByRole('button', { name: 'Close the page' }));

    tauri.setHandler('render_page', () => {
      throw { code: 'render_failed', message: "Atlas couldn't show this page." };
    });
    await user.click(cited.getByRole('button', { name: 'Show the page' }));
    const error = within(await pageDialog().findByTestId('page-error'));
    expect(error.getByText("Atlas couldn't show this page")).toBeVisible();
    await user.click(error.getByRole('button', { name: 'Open in the default app' }));
    expect(calls('open_original').at(-1)).toEqual({ fileId: 'file-1', allowDownload: false });
  });

  it('asks before downloading a file stored only in the cloud', async () => {
    tauri.setHandler('render_page', (args) => {
      if (!args?.allowDownload) {
        throw { code: 'file_cloud_only', message: 'This file is stored only in the cloud.' };
      }
      return tauri.pageImage();
    });
    const { user, cited } = await citedPassage();
    await user.click(cited.getByRole('button', { name: 'Show the page' }));
    const view = pageDialog();
    expect(
      await view.findByText(
        'This file is stored only in the cloud. Showing the page will download it.',
      ),
    ).toBeVisible();
    await user.click(view.getByRole('button', { name: 'Download and show' }));
    await waitFor(() => expect(calls('render_page').at(-1)).toMatchObject({ allowDownload: true }));
    expect(await view.findByTestId('page-image')).toBeVisible();
  });

  it('offers the page for a PDF result and a PDF bookmark, never an EPUB', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({
        hits: [
          tauri.searchHit({ fileInstanceId: 'h1' }),
          tauri.searchHit({
            fileInstanceId: 'h2',
            documentId: 'doc-2',
            displayName: 'gears.epub',
            fileType: 'epub',
            pageNo: null,
            location: 'Chapter 3, part 2',
          }),
        ],
        matchedDocuments: 2,
      }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'gears{Enter}');
    await h1('Search');
    const show = await screen.findByRole('button', {
      name: 'Show the page: harmonic.pdf, page 12',
    });
    expect(screen.queryByRole('button', { name: /Show the page: gears.epub/ })).toBeNull();
    await user.click(show);
    expect(calls('render_page').at(-1)).toMatchObject({ fileId: 'h1', pageNo: 12 });
    await user.click(pageDialog().getByRole('button', { name: 'Close the page' }));

    // A saved bookmark of a PDF page.
    tauri.setHandler('list_saved', () =>
      tauri.savedPage({
        items: [
          { item: 'bookmark', ...tauri.bookmark() },
          {
            item: 'bookmark',
            ...tauri.bookmark({
              id: 'bm-2',
              anchor: tauri.anchor({ title: 'Gears', fileType: 'epub', pageNo: null }),
            }),
          },
        ],
        total: 2,
      }),
    );
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(screen.getByRole('radio', { name: 'Saved' }));
    const items = await screen.findAllByTestId('saved-item');
    await user.click(
      within(items[0]).getByRole('button', { name: 'Show the page: Harmonic Drives' }),
    );
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith('record_open', {
        fileId: 'file-1',
        via: { kind: 'bookmark' },
        action: 'page',
        place: { pageNo: 12 },
      }),
    );
    expect(within(items[1]).queryByRole('button', { name: /Show the page/ })).toBeNull();
  });
});

describe('export and import', () => {
  beforeEach(() => {
    resetAll();
    libraryStore.reset();
  });

  const nav = () => within(screen.getByRole('navigation', { name: 'Main' }));
  const calls = (command: string) =>
    tauri.invoke.mock.calls.filter(([c]) => c === command).map(([, a]) => a);
  const exportDialog = () => within(screen.getByRole('dialog', { name: /^Export/ }) as HTMLElement);

  async function openTimeline() {
    tauri.setHandler('list_trails', () => ({
      trails: [tauri.trailSummary()],
      page: 0,
      pageSize: 25,
      total: 1,
    }));
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(await screen.findByRole('button', { name: 'harmonic drives' }));
    await screen.findByRole('heading', { level: 2, name: 'harmonic drives' });
    return user;
  }

  it('exports a trail as Markdown notes and says where it wrote them', async () => {
    const user = await openTimeline();
    const trigger = screen.getByRole('button', { name: 'Export…' });
    await user.click(trigger);
    const d = exportDialog();
    expect(d.getByRole('heading', { name: 'Export: harmonic drives' })).toBeVisible();
    expect(d.getByRole('radio', { name: 'This trail' })).toBeChecked();
    expect(d.getByRole('radio', { name: 'Markdown notes' })).toBeChecked();
    expect(d.getByRole('radio', { name: 'Obsidian links' })).toBeChecked();
    expect(d.getByRole('button', { name: 'Choose a folder and export…' })).toHaveFocus();
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(calls('export_knowledge').at(-1)).toEqual({
      scope: { kind: 'trail', id: 'trail-1' },
      format: 'markdown',
      linkStyle: 'obsidian',
    });
    expect(await d.findByTestId('export-result')).toHaveTextContent(
      '12 notes written to /Users/test/Notes/Atlas. 2 already existed and were left alone.',
    );
    await user.click(d.getByRole('button', { name: 'Done' }));
    expect(screen.queryByRole('dialog', { name: /^Export/ })).toBeNull();
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('exports everything as one file; link style is for notes only; cancel and errors', async () => {
    const user = await openTimeline();
    const trigger = screen.getByRole('button', { name: 'Export…' });
    await user.click(trigger);
    const d = exportDialog();
    await user.click(d.getByRole('radio', { name: 'Everything' }));
    expect(d.getByRole('heading', { name: 'Export everything' })).toBeVisible();
    expect(d.getByText('A complete copy in one file, which Atlas can import again.')).toBeVisible();
    await user.click(d.getByRole('radio', { name: 'One JSON file' }));
    expect(d.queryByRole('radio', { name: 'Obsidian links' })).toBeNull();

    // A cancelled folder dialog says nothing.
    tauri.setHandler('export_knowledge', () => null);
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(calls('export_knowledge').at(-1)).toEqual({
      scope: { kind: 'everything' },
      format: 'json',
      linkStyle: 'obsidian',
    });
    await waitFor(() =>
      expect(d.getByRole('button', { name: 'Choose a folder and export…' })).toBeEnabled(),
    );
    expect(d.queryByTestId('export-result')).toBeNull();

    // A refused folder says why.
    tauri.setHandler('export_knowledge', () => {
      throw tauri.apiError(
        'export_inside_library',
        "Atlas doesn't write into a library folder.",
        'Choose a folder outside your library folders.',
      );
    });
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(await d.findByText("Atlas doesn't write into a library folder.")).toBeVisible();
    expect(d.getByText('Choose a folder outside your library folders.')).toBeVisible();

    // One file written.
    tauri.setHandler('export_knowledge', () =>
      tauri.exportReport({ format: 'json', written: 1, besides: 0 }),
    );
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(await d.findByTestId('export-result')).toHaveTextContent(
      '1 file written to /Users/test/Notes/Atlas.',
    );

    // Escape closes and returns focus.
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: /^Export/ })).toBeNull();
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('offers Export on a finding and a collection, not on a bookmark', async () => {
    tauri.setHandler('list_saved', () =>
      tauri.savedPage({
        items: [
          { item: 'finding' as const, ...tauri.finding() },
          { item: 'bookmark' as const, ...tauri.bookmark({ id: 'bm-1' }) },
        ],
        total: 2,
      }),
    );
    tauri.setHandler('list_collections', () => [tauri.collection()]);
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Trails' }));
    await user.click(screen.getByRole('radio', { name: 'Saved' }));
    const items = await screen.findAllByTestId('saved-item');
    expect(within(items[1]).queryByRole('button', { name: 'Export…' })).toBeNull();
    await user.click(within(items[0]).getByRole('button', { name: 'Export…' }));
    let d = exportDialog();
    expect(d.getByRole('radio', { name: 'This finding' })).toBeChecked();
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(calls('export_knowledge').at(-1)).toMatchObject({
      scope: { kind: 'finding', id: 'f-1' },
    });
    await user.click(d.getByRole('button', { name: 'Done' }));

    await user.click(screen.getByRole('radio', { name: 'Interests and projects' }));
    await user.click(await screen.findByRole('button', { name: 'Mechanisms' }));
    await screen.findByRole('heading', { level: 2, name: 'Mechanisms' });
    await user.click(screen.getByRole('button', { name: 'Export…' }));
    d = exportDialog();
    expect(d.getByRole('heading', { name: 'Export: Mechanisms' })).toBeVisible();
    expect(d.getByRole('radio', { name: 'This collection' })).toBeChecked();
    await user.click(d.getByRole('radio', { name: 'Markdown links' }));
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(calls('export_knowledge').at(-1)).toEqual({
      scope: { kind: 'collection', id: 'c-1' },
      format: 'markdown',
      linkStyle: 'markdown',
    });
  });

  it('exports everything from Settings, and imports only after saying what it adds', async () => {
    const user = await launchOpen();
    await user.click(nav().getByRole('button', { name: 'Settings' }));
    const section = within(
      screen.getByRole('heading', { name: 'Export and import' }).closest('section') as HTMLElement,
    );
    await user.click(section.getByRole('button', { name: 'Export everything…' }));
    const d = exportDialog();
    expect(d.getByRole('heading', { name: 'Export everything' })).toBeVisible();
    expect(d.queryByRole('radio', { name: 'Everything' })).toBeNull();
    await user.click(d.getByRole('button', { name: 'Choose a folder and export…' }));
    expect(calls('export_knowledge').at(-1)).toMatchObject({ scope: { kind: 'everything' } });
    // Where it went: Rust shows the folder; the renderer sends nothing.
    await user.click(await d.findByRole('button', { name: /^Show in / }));
    expect(calls('reveal_export')).toEqual([undefined]);
    await user.click(d.getByRole('button', { name: 'Done' }));

    // A cancelled file dialog shows nothing.
    tauri.setHandler('preview_import', () => null);
    await user.click(section.getByRole('button', { name: 'Import from a file…' }));
    await waitFor(() =>
      expect(section.getByRole('button', { name: 'Import from a file…' })).toBeEnabled(),
    );
    expect(section.queryByTestId('import-preview')).toBeNull();

    // The dry run, then Cancel: nothing imported.
    tauri.setHandler('preview_import', () => tauri.importPreview());
    await user.click(section.getByRole('button', { name: 'Import from a file…' }));
    expect(await section.findByTestId('import-preview')).toHaveTextContent(
      'atlas-knowledge-2026-11-11.json: 3 trails, 14 findings and 20 bookmarks are new; 5 are already here.',
    );
    expect(section.getByText('Importing never removes anything.')).toBeVisible();
    await user.click(section.getByRole('button', { name: 'Cancel' }));
    expect(section.queryByTestId('import-preview')).toBeNull();
    expect(calls('apply_import')).toHaveLength(0);

    // Newer copies and trails expiry would take are said too; Import.
    tauri.setHandler('preview_import', () =>
      tauri.importPreview({
        summary: tauri.importSummary({ findings: { new: 0, newer: 1, here: 2 } }),
        expiring: 2,
      }),
    );
    await user.click(section.getByRole('button', { name: 'Import from a file…' }));
    const preview = await section.findByTestId('import-preview');
    expect(preview).toHaveTextContent(
      '3 trails and 20 bookmarks are new; 7 are already here. 1 here has a newer copy in the file and will be updated. 2 new trails are older than 90 days and will be removed by expiry unless pinned.',
    );
    await user.click(section.getByRole('button', { name: 'Import' }));
    expect(calls('apply_import').at(-1)).toEqual({ token: 'token-1' });
    expect(await section.findByTestId('import-result')).toHaveTextContent('Imported: 37 added.');

    // A file Atlas won't take says why.
    tauri.setHandler('preview_import', () => {
      throw tauri.apiError(
        'import_unknown_version',
        "That file isn't an Atlas export this version can read.",
        'Choose a file Atlas exported as One JSON file.',
      );
    });
    await user.click(section.getByRole('button', { name: 'Import from a file…' }));
    expect(
      await section.findByText("That file isn't an Atlas export this version can read."),
    ).toBeVisible();
  });
});

describe('diagnostics (Slice 4.1)', () => {
  beforeEach(resetAll);

  const calls = (cmd: string) =>
    tauri.invoke.mock.calls.filter(([c]) => c === cmd).map(([, args]) => args);

  async function openDiagnostics() {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const heading = await screen.findByRole('heading', { level: 2, name: 'Diagnostics' });
    return { user, section: within(heading.closest('section')!) };
  }

  it('says what a report holds and never holds, and that nothing is sent', async () => {
    const { section } = await openDiagnostics();
    expect(section.getByText(/Nothing is sent anywhere\./)).toBeInTheDocument();
    expect(section.getByRole('heading', { level: 3, name: 'A report holds' })).toBeVisible();
    expect(section.getByRole('heading', { level: 3, name: 'It never holds' })).toBeVisible();
    expect(
      section.getByText('What you searched for or asked, answers, or your notes'),
    ).toBeInTheDocument();
    expect(await section.findByTestId('log-size')).toHaveTextContent(
      'The log: 48.0 KB in 2 files.',
    );
  });

  it('saves a report, then shows it in the file manager', async () => {
    const { user, section } = await openDiagnostics();
    await user.click(section.getByRole('button', { name: 'Save a diagnostics report' }));
    expect(await section.findByTestId('report-saved')).toHaveTextContent('Report saved (52.0 KB).');
    await user.click(section.getByRole('button', { name: /^Show in / }));
    expect(calls('reveal_diagnostics')).toEqual([{ folder: 'reports' }]);
    await user.click(section.getByRole('button', { name: 'Show the log folder' }));
    expect(calls('reveal_diagnostics')).toEqual([{ folder: 'reports' }, { folder: 'log' }]);
  });

  it('says it is saving while it saves', async () => {
    tauri.setHandler('save_diagnostics_report', () => new Promise(() => {}));
    const { user, section } = await openDiagnostics();
    await user.click(section.getByRole('button', { name: 'Save a diagnostics report' }));
    expect(section.getByRole('button', { name: 'Saving…' })).toBeDisabled();
  });

  // Slice 4.1.1: one accidental click used to remove the whole log.
  it('clears the log only after asking', async () => {
    const { user, section } = await openDiagnostics();
    await section.findByTestId('log-size');
    await user.click(section.getByRole('button', { name: 'Clear the log' }));
    const confirm = within(section.getByRole('group', { name: 'Confirm clearing the log' }));
    expect(
      confirm.getByText('Clear the log? Its events can’t be brought back.'),
    ).toBeInTheDocument();
    expect(calls('clear_log')).toEqual([]);

    await user.click(confirm.getByRole('button', { name: 'Keep' }));
    expect(section.queryByRole('group', { name: 'Confirm clearing the log' })).toBeNull();
    expect(calls('clear_log')).toEqual([]);
    expect(section.getByTestId('log-size')).toHaveTextContent('The log: 48.0 KB in 2 files.');

    await user.click(section.getByRole('button', { name: 'Clear the log' }));
    await user.click(section.getByRole('button', { name: 'Clear' }));
    expect(calls('clear_log')).toEqual([undefined]);
    expect(await section.findByText('The log: 120 bytes in 1 file.')).toBeInTheDocument();
  });

  it('shows a report that failed, and a log that stopped', async () => {
    tauri.setHandler('get_diagnostics_status', () => tauri.diagnosticsStatus({ logStopped: true }));
    tauri.setHandler('save_diagnostics_report', () => {
      throw tauri.apiError(
        'diagnostics_unavailable',
        "Atlas couldn't save the diagnostics report.",
        'Check that the library’s drive has space and is connected, then try again.',
      );
    });
    const { user, section } = await openDiagnostics();
    expect(await section.findByText(/stopped writing its log/)).toBeInTheDocument();
    await user.click(section.getByRole('button', { name: 'Save a diagnostics report' }));
    expect(await section.findByText('Atlas couldn’t save the report')).toBeInTheDocument();
    expect(section.queryByTestId('report-saved')).not.toBeInTheDocument();
  });
});

describe('folder checks (Slice 4.5)', () => {
  beforeEach(resetAll);

  const books = () => tauri.sourceRoot({ id: 'root-books', displayPath: '/Users/test/Books' });
  const papers = () => tauri.sourceRoot({ id: 'root-papers', displayPath: '/Users/test/Papers' });
  const checked = (lastScan = tauri.scanRecord()) =>
    tauri.rootSummary({
      fileCount: 12,
      lastScanAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
      lastState: lastScan.state,
      lastScan,
    });
  const folders = () => screen.getByRole('list', { name: 'Library folders' });

  it('a folder row says what its last check found', async () => {
    tauri.setHandler('list_source_roots', () => [books()]);
    tauri.setHandler('get_inventory_status', () =>
      tauri.overview({
        roots: [checked(tauri.scanRecord({ newFiles: 3, changedFiles: 1, missingFiles: 2 }))],
      }),
    );
    await launchOpen();
    expect(screen.getByTestId('root-line')).toHaveTextContent('12 files · Checked 2 h ago');
    expect(screen.getByTestId('root-check-line')).toHaveTextContent(
      '3 new · 1 changed · 2 missing',
    );
  });

  it('a folder that is away says so, and what happens next', async () => {
    tauri.setHandler('list_source_roots', () => [
      tauri.sourceRoot({ displayPath: '/Volumes/Old/Papers', status: 'unavailable' }),
    ]);
    await launchOpen();
    expect(screen.getByTestId('root-away-line')).toHaveTextContent(
      "Not connected · Atlas checks it when it's back",
    );
    expect(within(folders()).queryByRole('button', { name: /Rescan/ })).not.toBeInTheDocument();
  });

  it('a scanning folder has its own Pause, Resume and Cancel, and no Rescan', async () => {
    tauri.setHandler('list_source_roots', () => [books(), papers()]);
    const active = tauri.inventoryStatus({ jobId: 'job-p', sourceRootId: 'root-papers' });
    tauri.setHandler('get_inventory_status', () => tauri.overview({ active }));
    const user = await launchOpen();
    const list = within(folders());
    expect(list.getByRole('button', { name: 'Rescan Books' })).toBeInTheDocument();
    expect(list.queryByRole('button', { name: 'Rescan Papers' })).not.toBeInTheDocument();
    expect(list.queryByRole('button', { name: 'Pause cataloging Books' })).not.toBeInTheDocument();

    await user.click(list.getByRole('button', { name: 'Pause cataloging Papers' }));
    expect(tauri.invoke).toHaveBeenCalledWith('pause_job', { jobId: 'job-p' });

    events.emit(
      'inventory-status',
      tauri.overview({ active: { ...active, state: 'paused_user' } }),
    );
    await user.click(await list.findByRole('button', { name: 'Resume cataloging Papers' }));
    expect(tauri.invoke).toHaveBeenCalledWith('resume_job', { jobId: 'job-p' });
    await user.click(list.getByRole('button', { name: 'Cancel cataloging Papers' }));
    expect(tauri.invoke).toHaveBeenCalledWith('cancel_job', { jobId: 'job-p' });
  });

  it('Check all folders now asks Atlas to check every folder', async () => {
    tauri.setHandler('list_source_roots', () => [books()]);
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Check all folders now' }));
    expect(tauri.invoke).toHaveBeenCalledWith('check_all_folders', undefined);
  });

  it('a control that fails says why, and stays said after the refresh', async () => {
    tauri.setHandler('list_source_roots', () => [books()]);
    tauri.setHandler('get_inventory_status', () =>
      tauri.overview({ active: tauri.inventoryStatus() }),
    );
    tauri.setHandler('pause_job', () => {
      throw tauri.apiError('job_not_pausable', 'This job has already finished.', 'Try again.');
    });
    const user = await launchOpen();
    await user.click(within(folders()).getByRole('button', { name: 'Pause cataloging Books' }));
    expect(await screen.findByText('This job has already finished.')).toBeInTheDocument();
    await waitFor(() =>
      expect(
        tauri.invoke.mock.calls.filter(([c]) => c === 'get_inventory_status').length,
      ).toBeGreaterThan(1),
    );
    expect(screen.getByText('This job has already finished.')).toBeInTheDocument();
  });

  it('Recent checks lists each check when opened', async () => {
    tauri.setHandler('list_source_roots', () => [books()]);
    tauri.setHandler('get_inventory_status', () => tauri.overview({ roots: [checked()] }));
    const finished = new Date(Date.now() - 3 * 3_600_000).toISOString();
    tauri.setHandler('list_scans', () => [
      tauri.scanRecord({ jobId: 'scan-2', finishedAt: finished, newFiles: 4 }),
      tauri.scanRecord({
        jobId: 'scan-1',
        origin: 'added',
        state: 'cancelled',
        finishedAt: finished,
      }),
    ]);
    const user = await launchOpen();
    await user.click(screen.getByText('Recent checks'));
    const history = await screen.findByRole('list', { name: 'Recent checks of Books' });
    const items = within(history).getAllByRole('listitem');
    expect(items[0]).toHaveTextContent('3 h ago · automatically · 2 s · 4 new');
    expect(items[1]).toHaveTextContent('3 h ago · when you added it · stopped by you');
    expect(tauri.invoke).toHaveBeenCalledWith('list_scans', {
      sourceRootId: 'root-books',
      limit: 10,
    });
  });

  it('Couldn’t read lists each folder with what to do', async () => {
    tauri.setHandler('list_source_roots', () => [books()]);
    tauri.setHandler('get_inventory_status', () =>
      tauri.overview({ roots: [checked(tauri.scanRecord({ jobId: 'scan-9', problems: 1 }))] }),
    );
    tauri.setHandler('get_scan_problems', () => [
      { displayPath: '/Users/test/Books/Private', code: 'permission_denied' },
    ]);
    const user = await launchOpen();
    expect(screen.getByTestId('root-check-line')).toHaveTextContent(
      "No changes · Couldn't read 1 folder",
    );
    await user.click(screen.getByText("Couldn't read 1 folder", { selector: 'summary' }));
    expect(await screen.findByText('/Users/test/Books/Private')).toBeInTheDocument();
    expect(
      screen.getByText(/Atlas isn't allowed to read it\. .*then Rescan\./),
    ).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith('get_scan_problems', { jobId: 'scan-9' });
  });

  it('the status bar names Atlas’s own check and lists every folder’s activity', async () => {
    tauri.setHandler('list_source_roots', () => [books(), papers()]);
    const check = tauri.inventoryStatus({ origin: 'automatic' });
    const other = tauri.inventoryStatus({
      jobId: 'job-p',
      sourceRootId: 'root-papers',
      sourceRootDisplayPath: '/Users/test/Papers',
      state: 'queued',
    });
    tauri.setHandler('get_inventory_status', () =>
      tauri.overview({ active: check, activeJobs: [check, other] }),
    );
    tauri.setHandler('get_extraction_status', () =>
      tauri.extractionOverview({
        roots: [
          {
            sourceRootId: 'root-books',
            counts: tauri.extractionCounts(),
            lastState: null,
            activeJobId: 'extract-1',
            activeState: 'running',
            failedBecause: null,
          },
        ],
      }),
    );
    const user = await launchOpen();
    expect(screen.getByTestId('inventory-status')).toHaveTextContent('Checking Books for changes');
    const all = screen.getByTestId('all-activity');
    await user.click(within(all).getByText('All activity'));
    const rows = within(all).getAllByRole('listitem');
    expect(rows[0]).toHaveTextContent('Books Checking for changes · Reading text');
    expect(rows[1]).toHaveTextContent('Papers Waiting to catalog');
  });

  it('Settings chooses how often folders are checked, in words', async () => {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const group = await screen.findByRole('group', { name: 'Check folders for changes' });
    expect(within(group).getByRole('radio', { name: 'Every 6 hours' })).toBeChecked();
    expect(screen.getByTestId('check-folders-note')).toHaveTextContent('every 6 hours');
    await user.click(within(group).getByRole('radio', { name: 'Only when I ask' }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith(
        'update_settings',
        expect.objectContaining({
          settings: expect.objectContaining({ background: { checkFolders: 'never' } }),
        }),
      ),
    );
    expect(screen.getByTestId('check-folders-note')).toHaveTextContent(
      'only when you choose Rescan or Check all folders now',
    );
  });
});

describe('one book, two formats (Slice 4.5.1)', () => {
  beforeEach(resetAll);

  const pdf = {
    fileInstanceId: 'file-pdf',
    fileType: 'pdf' as const,
    displayName: 'Dune.pdf',
    displayPath: '/Users/test/Books/Dune.pdf',
    sourceRootId: 'root-books',
    availability: 'available' as const,
    storage: 'local' as const,
  };

  it('a search result names both formats, each with its own Open', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('search_documents', () =>
      tauri.searchPage({
        hits: [
          tauri.searchHit({
            fileInstanceId: 'file-epub',
            fileType: 'epub',
            displayName: 'Dune.epub',
            otherFormat: pdf,
          }),
        ],
        matchedDocuments: 1,
      }),
    );
    const user = await launchOpen();
    await user.type(screen.getByLabelText('Search your library'), 'spice{Enter}');
    await h1('Search');
    const hit = await screen.findByTestId('search-hit');
    expect(hit).toHaveTextContent('EPUB · PDF');
    expect(within(hit).getByTestId('other-format')).toHaveTextContent('Also as PDF');
    expect(within(hit).getByRole('button', { name: 'Open Dune.epub' })).toBeInTheDocument();
    await user.click(within(hit).getByRole('button', { name: 'Open Dune.pdf' }));
    expect(tauri.invoke).toHaveBeenCalledWith(
      'open_original',
      expect.objectContaining({ fileId: 'file-pdf' }),
    );
  });

  it('the Library lists the book once, as EPUB · PDF', async () => {
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('list_documents', () =>
      tauri.documentPage({
        items: [
          tauri.fileInstance({
            id: 'file-epub',
            displayName: 'Dune.epub',
            fileType: 'epub',
            extension: 'epub',
            otherFormat: pdf,
          }),
        ],
        matchedSoFar: 1,
      }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    const rows = within(await screen.findByRole('table')).getAllByRole('row');
    expect(rows).toHaveLength(2);
    expect(rows[1]).toHaveTextContent('EPUB · PDF');
    expect(within(rows[1]).getByTestId('other-format')).toHaveTextContent('Also as PDF');
  });

  it('the panel says why, and Not the same book keeps them apart', async () => {
    const epubFile = tauri.fileInstance({
      id: 'file-epub',
      displayName: 'Dune.epub',
      fileType: 'epub',
      extension: 'epub',
      documentId: 'doc-epub',
      extraction: { state: 'done', reason: null },
    });
    const partner = tauri.fileInstance({
      id: 'file-pdf',
      displayName: 'Dune.pdf',
      documentId: 'doc-pdf',
      extraction: { state: 'done', reason: null },
    });
    const detail = (state: 'same' | 'different', basis: 'text' | 'owner') =>
      tauri.documentDetail({
        file: epubFile,
        documentId: 'doc-epub',
        formats: { partner, state, basis, indexedAs: state === 'same' ? 'epub' : null },
      });
    tauri.setHandler('get_document_detail', () => detail('same', 'text'));
    tauri.setHandler('set_same_book', (args) => detail(args?.same ? 'same' : 'different', 'owner'));
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('list_documents', () =>
      tauri.documentPage({ items: [epubFile], matchedSoFar: 1 }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await user.click(await screen.findByRole('button', { name: 'Details for Dune.epub' }));
    const section = await screen.findByTestId('format-pair');
    expect(within(section).getByTestId('format-note')).toHaveTextContent(
      'The same book as the PDF beside it. Atlas compared their text. Searched by meaning through the EPUB.',
    );
    await user.click(within(section).getByRole('button', { name: 'Not the same book' }));
    expect(tauri.invoke).toHaveBeenCalledWith('set_same_book', {
      fileId: 'file-epub',
      same: false,
    });
    expect(await screen.findByText(/You said they are different books/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Same book' }));
    expect(tauri.invoke).toHaveBeenCalledWith('set_same_book', { fileId: 'file-epub', same: true });
  });

  it('a pair not yet read says it waits, with nothing to choose', async () => {
    const partner = tauri.fileInstance({ id: 'file-pdf', displayName: 'Dune.pdf' });
    tauri.setHandler('get_document_detail', () =>
      tauri.documentDetail({
        formats: { partner, state: 'waiting', basis: null, indexedAs: null },
      }),
    );
    tauri.setHandler('list_source_roots', () => [tauri.sourceRoot()]);
    tauri.setHandler('list_documents', () =>
      tauri.documentPage({ items: [tauri.fileInstance()], matchedSoFar: 1 }),
    );
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    await user.click(await screen.findByRole('button', { name: 'Details for book.pdf' }));
    const section = await screen.findByTestId('format-pair');
    expect(section).toHaveTextContent('Atlas will compare them once it has read both.');
    expect(within(section).queryByRole('button', { name: /same book/i })).not.toBeInTheDocument();
  });
});
