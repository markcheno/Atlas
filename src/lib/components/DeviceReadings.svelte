<script lang="ts">
  import type { DeviceSnapshot } from '../api/bindings/DeviceSnapshot';
  import { coresText, dataDriveText, memoryText, powerText } from '../deviceCopy';

  let { snapshot }: { snapshot: DeviceSnapshot } = $props();
</script>

<dl class="readings" aria-label="Device readings">
  <div>
    <dt>Memory</dt>
    <dd>{memoryText(snapshot.totalMemoryBytes)}</dd>
  </div>
  <div>
    <dt>Processor</dt>
    <dd>{coresText(snapshot.logicalCores)}</dd>
  </div>
  <div>
    <dt>Atlas data drive</dt>
    <dd>{dataDriveText(snapshot)}</dd>
  </div>
  <div>
    <dt>Power</dt>
    <dd>{powerText(snapshot)}</dd>
  </div>
</dl>

<style>
  .readings {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: var(--space-4);
    margin: 0;
    padding: var(--space-4) var(--space-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  dt {
    color: var(--text-muted);
    font-size: 13px;
  }

  dd {
    margin: 0;
    font-weight: 500;
  }
</style>
