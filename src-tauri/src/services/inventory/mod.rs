//! Read-only traversal of source roots: streams immediate directory
//! entries in bounded batches, classifies PDF/EPUB by extension and
//! lightweight signature, skips symlinks, and excludes the data root.
//! Never reads full file contents or writes to source folders.
//!
//! Not implemented in Slice 0.1. Delivered by Slice 0.5.
