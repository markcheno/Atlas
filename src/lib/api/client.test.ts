import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as tauriMock from '../../test/tauriMock';

vi.mock('@tauri-apps/api/core', () => import('../../test/tauriMock'));

const { atlas, AtlasApiError, toApiError } = await import('./client');

describe('atlas IPC facade', () => {
  beforeEach(() => {
    tauriMock.isTauri.mockReturnValue(true);
    tauriMock.resetHandlers();
  });

  it('invokes commands by their Rust names with camelCase arguments', async () => {
    const status = await atlas.getAppStatus();
    expect(status.slice).toBe('0.3');
    expect(tauriMock.invoke).toHaveBeenCalledWith('get_app_status', undefined);

    await atlas.startInventory('root-1').catch(() => undefined);
    expect(tauriMock.invoke).toHaveBeenCalledWith('start_inventory', { sourceRootId: 'root-1' });
  });

  it('maps a Rust error payload to AtlasApiError', async () => {
    const err = await atlas
      .listDocuments({
        text: null,
        sourceRootId: null,
        fileType: null,
        sort: 'name_asc',
        page: 0,
        pageSize: 50,
      })
      .then(
        () => null,
        (e: unknown) => e,
      );
    expect(err).toBeInstanceOf(AtlasApiError);
    const apiErr = err as InstanceType<typeof AtlasApiError>;
    expect(apiErr.code).toBe('not_implemented');
    expect(apiErr.isNotImplemented).toBe(true);
    expect(apiErr.command).toBe('list_documents');
    expect(apiErr.plannedSlice).toBe('0.6');
  });

  it('reports ipc_unavailable outside the desktop app without invoking', async () => {
    tauriMock.isTauri.mockReturnValue(false);
    tauriMock.invoke.mockClear();
    await expect(atlas.getAppStatus()).rejects.toMatchObject({ code: 'ipc_unavailable' });
    expect(tauriMock.invoke).not.toHaveBeenCalled();
  });

  it('never surfaces raw unknown errors', () => {
    const err = toApiError('get_app_status', new Error('/Users/someone/secret/path'));
    expect(err.code).toBe('unexpected');
    expect(err.message).not.toContain('/Users/');
  });
});
