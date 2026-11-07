import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => import('./test/tauriMock'));

const { default: App } = await import('./App.svelte');
const { view } = await import('./lib/stores/view.svelte');
const { theme } = await import('./lib/stores/theme.svelte');

async function startApp() {
  const user = userEvent.setup();
  render(App);
  await user.click(screen.getByRole('button', { name: 'Get Started' }));
  return user;
}

describe('Atlas shell', () => {
  beforeEach(() => {
    view.go('welcome');
    theme.set('system');
  });

  it('opens on Welcome with one primary action and no sign-in or AI gate', () => {
    render(App);
    expect(
      screen.getByRole('heading', { level: 1, name: "Explore everything you've collected." }),
    ).toHaveFocus();
    expect(screen.getAllByRole('button')).toHaveLength(1);
    expect(screen.queryByText(/sign in/i)).not.toBeInTheDocument();
  });

  it('Get Started leads to Home with main navigation', async () => {
    await startApp();
    expect(
      screen.getByRole('heading', { level: 1, name: 'What are you curious about?' }),
    ).toHaveFocus();
    const nav = screen.getByRole('navigation', { name: 'Main' });
    expect(nav).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Home' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByText('Not cataloging yet')).toBeInTheDocument();
  });

  it('Library shows the honest not-implemented state and no fabricated rows', async () => {
    const user = await startApp();
    await user.click(screen.getByRole('button', { name: 'Library' }));
    expect(screen.getByRole('heading', { level: 1, name: 'Library' })).toBeInTheDocument();
    expect(await screen.findByText(/planned for Phase 0 slice 0\.6/)).toBeInTheDocument();
    expect(
      screen.getByText('Choose a folder to start discovering your library.'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(screen.queryByText(/files cataloged/)).not.toBeInTheDocument();
  });

  it('keyboard shortcuts switch views', async () => {
    const user = await startApp();
    await user.keyboard('{Control>}3{/Control}');
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toHaveFocus();
    await user.keyboard('{Control>}2{/Control}');
    expect(screen.getByRole('heading', { level: 1, name: 'Library' })).toHaveFocus();
  });

  it('theme choice overrides the system appearance', async () => {
    const user = await startApp();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('radio', { name: 'Dark' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
    await user.click(screen.getByRole('radio', { name: 'Match system' }));
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
