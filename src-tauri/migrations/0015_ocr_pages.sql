-- Phase 4, Slice 4.1: what a document that needs OCR is known to hold
-- (phase4-spec §6.1, §7.9). The PDF reader now reports a Needs OCR
-- document's page count and its pages with usable text.
ALTER TABLE documents ADD COLUMN text_pages INTEGER;

-- PDFs marked Needs OCR before this version have neither count. They are
-- read once more to get them (they aren't searchable, so nothing is lost
-- while they wait). EPUBs have no pages to count and stay as they are.
UPDATE documents SET refresh = 1
 WHERE state = 'needs_ocr'
   AND page_count IS NULL
   AND EXISTS (SELECT 1 FROM file_instances f
                WHERE f.document_id = documents.id AND f.file_type = 'pdf');
