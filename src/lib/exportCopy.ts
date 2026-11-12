// Export and import wording (Slice 3.8, phase3-spec §5.8).
import type { ExportReport } from './api/bindings/ExportReport';
import type { ImportPreview } from './api/bindings/ImportPreview';
import type { ImportSummary } from './api/bindings/ImportSummary';
import { count } from './inventoryCopy';

export const EXPORT = 'Export…';
export const EXPORT_TITLE = 'Export';
export const EXPORT_EVERYTHING = 'Export everything…';
export const EXPORT_EVERYTHING_TITLE = 'Export everything';
export const EVERYTHING = 'Everything';
export const MARKDOWN_NOTES = 'Markdown notes';
export const ONE_JSON_FILE = 'One JSON file';
export const MARKDOWN_NOTES_NOTE =
  'A note for each trail, finding, bookmarked document, interest and project, for a notes app such as Obsidian.';
export const ONE_JSON_FILE_NOTE = 'A complete copy in one file, which Atlas can import again.';
export const OBSIDIAN_LINKS = 'Obsidian links';
export const MARKDOWN_LINKS = 'Markdown links';
export const CHOOSE_AND_EXPORT = 'Choose a folder and export…';
export const EXPORTING = 'Exporting…';
export const EXPORT_NOTE =
  'Next, you choose a folder. Atlas writes into a folder called Atlas inside it, and never changes notes of your own there.';
export const EXPORT_IMPORT_TITLE = 'Export and import';
export const EXPORT_IMPORT_NOTE =
  'Export everything takes your trails, bookmarks, findings, interests and projects out of Atlas, into a folder called Atlas inside a folder you choose: as Markdown notes, or as one JSON file that can be imported again, here or into another library. A single trail, finding, interest or project has its own Export… too.';
export const IMPORT_FROM_FILE = 'Import from a file…';
export const IMPORT = 'Import';
export const IMPORTING = 'Importing…';
export const IMPORT_NEVER_REMOVES = 'Importing never removes anything.';

export type ExportWhat = 'trail' | 'finding' | 'collection';

/** "This trail", "This finding", "This collection". */
export function thisItem(what: ExportWhat): string {
  return `This ${what}`;
}

/**
 * "12 notes written to …/Atlas. 2 already existed and were left alone."
 */
export function exportResult(r: ExportReport): string {
  const files = r.written + r.replaced;
  const what = r.format === 'json' ? count(files, 'file') : count(files, 'note');
  let text = `${what} written to ${r.folder}.`;
  if (r.replaced > 0 && r.format === 'markdown') {
    text += ` ${count(r.replaced, 'note')} from an earlier export ${r.replaced === 1 ? 'was' : 'were'} replaced.`;
  }
  if (r.besides > 0) {
    text += ` ${r.besides.toLocaleString('en-US')} already existed and ${r.besides === 1 ? 'was' : 'were'} left alone.`;
  }
  return text;
}

function join(parts: string[]): string {
  if (parts.length <= 1) return parts.join('');
  return `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
}

function kinds(summary: ImportSummary, pick: (c: ImportSummary['trails']) => number): string[] {
  const parts: string[] = [];
  const add = (n: number, one: string, many?: string) => {
    if (n > 0) parts.push(count(n, one, many));
  };
  add(pick(summary.trails), 'trail');
  add(pick(summary.findings), 'finding');
  add(pick(summary.bookmarks), 'bookmark');
  add(pick(summary.collections), 'interest or project', 'interests and projects');
  return parts;
}

function total(summary: ImportSummary, pick: (c: ImportSummary['trails']) => number): number {
  return (
    pick(summary.trails) +
    pick(summary.findings) +
    pick(summary.bookmarks) +
    pick(summary.collections)
  );
}

/**
 * "3 trails, 14 findings and 20 bookmarks are new; 5 are already here."
 */
export function importPreviewText(p: ImportPreview): string {
  const fresh = kinds(p.summary, (c) => c.new);
  const newer = total(p.summary, (c) => c.newer);
  const here = total(p.summary, (c) => c.here);
  const sentences: string[] = [];
  const isAre = (n: number) => (n === 1 ? 'is' : 'are');
  let first =
    fresh.length > 0
      ? `${join(fresh)} ${isAre(total(p.summary, (c) => c.new))} new`
      : 'Nothing in it is new';
  if (here > 0) first += `; ${here.toLocaleString('en-US')} ${isAre(here)} already here`;
  sentences.push(`${first}.`);
  if (newer > 0) {
    sentences.push(
      `${newer.toLocaleString('en-US')} here ${newer === 1 ? 'has' : 'have'} a newer copy in the file and will be updated.`,
    );
  }
  if (p.expiring > 0) {
    sentences.push(
      `${count(p.expiring, 'new trail')} ${p.expiring === 1 ? 'is' : 'are'} older than 90 days and will be removed by expiry unless pinned.`,
    );
  }
  return sentences.join(' ');
}

/** What an import did. */
export function importResult(summary: ImportSummary): string {
  const added = total(summary, (c) => c.new);
  const updated = total(summary, (c) => c.newer);
  if (added === 0 && updated === 0) return 'Imported. Everything in it was already here.';
  const parts: string[] = [];
  if (added > 0) parts.push(`${added.toLocaleString('en-US')} added`);
  if (updated > 0) parts.push(`${updated.toLocaleString('en-US')} updated`);
  return `Imported: ${join(parts)}.`;
}
