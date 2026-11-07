/**
 * Startup routing. The Rust core reopens the remembered data root before
 * the window loads; the UI asks for the result and picks the first view.
 */
import { atlas } from './api/client';
import type { DataRootStatus } from './api/bindings/DataRootStatus';
import { theme } from './stores/theme.svelte';
import { view } from './stores/view.svelte';
import { workspace } from './stores/workspace.svelte';

/** Records a new data-root status and moves to the matching view. */
export async function applyStatus(status: DataRootStatus) {
  workspace.status = status;
  if (status.state === 'open') {
    await theme.load();
    view.go('home');
  } else if (status.state === 'not_configured') {
    view.go('welcome');
  } else {
    view.go('unavailable');
  }
}

export async function boot() {
  try {
    await applyStatus(await atlas.getDataRootStatus());
  } catch {
    // Outside the desktop app (plain `pnpm dev`): show onboarding only.
    workspace.status = null;
    view.go('welcome');
  }
}
