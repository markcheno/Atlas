/** Facts about the running app (OS, version), loaded once at boot. */
import { atlas } from '../api/client';
import type { AppStatus } from '../api/bindings/AppStatus';

class AppInfo {
  status = $state<AppStatus | null>(null);

  async load() {
    try {
      this.status = await atlas.getAppStatus();
    } catch {
      this.status = null;
    }
  }
}

export const appInfo = new AppInfo();
