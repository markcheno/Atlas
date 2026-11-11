-- Phase 3, Slice 3.1: how much text the reader left out as Project
-- Gutenberg's header and licence (EPUB reader version 3, phase3-spec §7.6).
ALTER TABLE documents ADD COLUMN boilerplate_chars INTEGER NOT NULL DEFAULT 0;
