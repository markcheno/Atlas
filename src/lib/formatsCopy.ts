/**
 * Words for one book in two formats (Slice 4.5.1, ADR 0023): a PDF and an
 * EPUB beside each other with one name, whose texts match, are one book.
 */
import type { FileType } from './api/bindings/FileType';
import type { FormatFile } from './api/bindings/FormatFile';
import type { FormatPair } from './api/bindings/FormatPair';
import type { OpenVia } from './api/bindings/OpenVia';
import type { Original } from './stores/originals.svelte';

const NAME: Record<FileType, string> = { pdf: 'PDF', epub: 'EPUB' };

/** "EPUB" or, for a book in both formats, "EPUB · PDF" (the shown one first). */
export function typesText(shown: FileType, other: FormatFile | null | undefined): string {
  return other ? `${NAME[shown]} · ${NAME[other.fileType]}` : NAME[shown];
}

/** "Also as PDF". */
export const alsoAs = (other: FormatFile) => `Also as ${NAME[other.fileType]}`;

export const NOT_SAME = 'Not the same book';
export const SAME = 'Same book';

/** The detail panel's line about the two formats. */
export function pairNote(pair: FormatPair): string {
  const other = NAME[pair.partner.fileType];
  switch (pair.state) {
    case 'waiting':
      return `Also here as ${other}. Atlas will compare them once it has read both.`;
    case 'different':
      return pair.basis === 'owner'
        ? `Also here as ${other}, with the same name. You said they are different books.`
        : `Also here as ${other}, with the same name, but their texts differ, so Atlas keeps them apart.`;
    case 'same': {
      const how =
        pair.basis === 'text'
          ? 'Atlas compared their text.'
          : pair.basis === 'name'
            ? `Atlas couldn't read one of them, so it went by the name.`
            : 'You said so.';
      const indexed = pair.indexedAs
        ? ` Searched by meaning through the ${NAME[pair.indexedAs]}.`
        : '';
      return `The same book as the ${other} beside it. ${how}${indexed}`;
    }
  }
}

/** A book's other format as something to open or show. */
export const fromFormat = (f: FormatFile, via?: OpenVia): Original => ({
  id: f.fileInstanceId,
  displayName: f.displayName,
  availability: f.availability,
  via,
});
