// Diagnostics wording (Slice 4.1, phase4-spec §5.8).
import type { DiagnosticsStatus } from './api/bindings/DiagnosticsStatus';
import type { ReportSaved } from './api/bindings/ReportSaved';
import { formatBytes } from './format';

export const DIAGNOSTICS_TITLE = 'Diagnostics';
export const DIAGNOSTICS_NOTE =
  'Atlas keeps a log of what it did, in counts and timings, on this computer. Nothing is sent anywhere.';
export const SAVE_REPORT = 'Save a diagnostics report';
export const SAVING = 'Saving…';
export const SHOW_LOG = 'Show the log folder';
export const CLEAR_LOG = 'Clear the log';
export const CLEARING = 'Clearing…';
export const LOG_STOPPED =
  'Atlas stopped writing its log after a failed write. It starts again when the library is opened again.';

/** What a report holds. */
export const REPORT_HOLDS = [
  'Versions of Atlas, your system and its parts',
  'This computer’s readings: memory, power, free space',
  'How many files, documents, pages and passages, by status, and their sizes',
  'How many trails, findings, bookmarks, interests and projects',
  'Which settings are on and off',
  'The log’s last events: what ran, how long it took, and error codes',
];

/** What a report never holds. */
export const REPORT_NEVER_HOLDS = [
  'Names of files or folders, or where they are',
  'Titles, authors or any text from your documents',
  'What you searched for or asked, answers, or your notes',
  'Names or addresses of AI services or models, or any key',
];

export const REPORT_SHARE_NOTE =
  'The report stays on this computer, in the library’s exports folder. Share it only if you choose to.';

/** "The log: 48 KB in 2 files." */
export function logLine(s: DiagnosticsStatus): string {
  if (s.logFiles === 0) return 'The log is empty.';
  const files = s.logFiles === 1 ? '1 file' : `${s.logFiles.toLocaleString('en-US')} files`;
  return `The log: ${formatBytes(s.logBytes)} in ${files}.`;
}

/** "Report saved (52 KB)." */
export function savedText(r: ReportSaved): string {
  return `Report saved (${formatBytes(r.bytes)}).`;
}
