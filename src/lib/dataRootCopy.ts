/** Plain-language copy for data-root locations and their problems. */
import type { CandidateKind } from './api/bindings/CandidateKind';
import type { CandidateProblem } from './api/bindings/CandidateProblem';
import type { DataRootCandidate } from './api/bindings/DataRootCandidate';
import { formatBytes } from './format';

export const KIND_TEXT: Record<CandidateKind, string> = {
  new_folder: 'Atlas will create this folder.',
  empty_folder: 'This folder is empty and ready to use.',
  existing_library: 'An Atlas library is already here. Atlas will open it.',
  incomplete_library:
    'Atlas started setting up a library here before but did not finish. It will complete the setup.',
  non_empty_folder:
    'This folder already contains other files. Atlas keeps its data in a folder of its own.',
  not_a_folder: 'This is a file, not a folder.',
};

export function problemText(problem: CandidateProblem, candidate: DataRootCandidate): string {
  switch (problem) {
    case 'not_writable':
      return 'Atlas cannot write to this location.';
    case 'network_volume':
      return 'This is a network drive. Atlas needs a folder on this computer or on a directly connected drive.';
    case 'cloud_synced':
      return 'This folder is synced to the cloud. Syncing can damage the Atlas catalog, so choose a folder outside it.';
    case 'low_space':
      return `Not enough free space. Atlas needs at least ${formatBytes(candidate.requiredFreeBytes)} free to start a library here.`;
    case 'newer_schema':
      return 'This library was made by a newer version of Atlas. Update Atlas to open it.';
    case 'invalid_manifest':
      return 'The information file in this library is damaged.';
    case 'inside_library':
      return 'This folder is inside another Atlas library.';
  }
}
