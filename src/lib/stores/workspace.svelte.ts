/** The Rust core's view of the Atlas data root, mirrored for the UI. */
import type { DataRootStatus } from '../api/bindings/DataRootStatus';

class WorkspaceState {
  status = $state<DataRootStatus | null>(null);

  get isOpen(): boolean {
    return this.status?.state === 'open';
  }
}

export const workspace = new WorkspaceState();
