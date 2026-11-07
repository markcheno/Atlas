import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as tauri from './test/tauriMock';

vi.mock('@tauri-apps/api/core', () => import('./test/tauriMock'));

const { default: App } = await import('./App.svelte');
const { view } = await import('./lib/stores/view.svelte');
const { theme } = await import('./lib/stores/theme.svelte');
const { workspace } = await import('./lib/stores/workspace.svelte');

const h1 = (name: string | RegExp) => screen.findByRole('heading', { level: 1, name });

/** Fresh install: Welcome, then accept the suggested location. */
async function onboard() {
  const user = userEvent.setup();
  render(App);
  await user.click(await screen.findByRole('button', { name: 'Get Started' }));
  await user.click(await screen.findByRole('button', { name: 'Use this location' }));
  await h1('What are you curious about?');
  return user;
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
  beforeEach(() => {
    tauri.resetHandlers();
    view.go('loading');
    workspace.status = null;
    theme.apply('system');
  });

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
  beforeEach(() => {
    tauri.resetHandlers();
    view.go('loading');
    workspace.status = null;
    theme.apply('system');
  });

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
  beforeEach(() => {
    tauri.resetHandlers();
    view.go('loading');
    workspace.status = null;
    theme.apply('system');
  });

  it('shows navigation and status after onboarding', async () => {
    await onboard();
    expect(screen.getByRole('navigation', { name: 'Main' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Home' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByText('Not cataloging yet')).toBeInTheDocument();
  });

  it('Library shows the honest not-implemented state and no fabricated rows', async () => {
    const user = await launchOpen();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    expect(await screen.findByText(/planned for Phase 0 slice 0\.6/)).toBeInTheDocument();
    expect(
      screen.getByText('Choose a folder to start discovering your library.'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(screen.queryByText(/files cataloged/)).not.toBeInTheDocument();
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
