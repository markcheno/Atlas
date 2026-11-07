/**
 * Startup routing. The Rust core reopens the remembered data root before
 * the window loads; the UI asks for the result and picks the first view.
 */
import { atlas } from './api/client';
import type { DataRootStatus } from './api/bindings/DataRootStatus';
import { appInfo } from './stores/app.svelte';
import { inventory } from './stores/inventory.svelte';
import { library } from './stores/library.svelte';
import { sources } from './stores/sources.svelte';
import { theme } from './stores/theme.svelte';
import { view } from './stores/view.svelte';
import { workspace } from './stores/workspace.svelte';

/**
 * Records a new data-root status and moves to the matching view. After the
 * user has just chosen a location, a library with no folders continues to
 * the Library folders step of onboarding.
 */
export async function applyStatus(status: DataRootStatus, opts: { justChosen?: boolean } = {}) {
  workspace.status = status;
  library.reset();
  if (!appInfo.status) void appInfo.load();
  if (status.state === 'open') {
    await Promise.all([
      theme.load(),
      sources.refresh(),
      inventory.refresh(),
      inventory.subscribe(),
    ]);
    view.go(opts.justChosen && sources.roots.length === 0 ? 'library-folders' : 'home');
  } else if (status.state === 'not_configured') {
    sources.clear();
    inventory.clear();
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
