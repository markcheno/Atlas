/** Wording for the page view (Slice 3.7, phase3-spec §5.5). */

export const SHOW_PAGE = 'Show the page';
export const COULD_NOT_SHOW = "Atlas couldn't show this page";
export const CLOUD_ONLY =
  'This file is stored only in the cloud. Showing the page will download it.';
export const DOWNLOAD_AND_SHOW = 'Download and show';
export const OPEN_DEFAULT = 'Open in the default app';
export const FIT_WIDTH = 'Fit width';
export const ACTUAL_SIZE = 'Actual size';
export const BOOKMARK_PAGE = 'Bookmark this page';
export const PAGE_TEXT = "The page's text, as Atlas read it";
export const NOT_FOUND_ON_PAGE = "The passage isn't marked: Atlas didn't find it on this page.";

export function showing(page: number): string {
  return `Showing page ${page}…`;
}

/** "Page 12 of 340". */
export function pageOf(page: number, count: number | null): string {
  return count ? `Page ${page} of ${count.toLocaleString('en-US')}` : `Page ${page}`;
}
