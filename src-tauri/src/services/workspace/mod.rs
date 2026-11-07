//! Atlas Data Root lifecycle: OS-default location, `atlas.json` manifest
//! (root UUID, layout and schema versions, written atomically), the
//! OS-managed locator file, the per-root writer lock, and rejection of
//! network or cloud-synced volumes.
//!
//! Not implemented in Slice 0.1. Delivered by Slice 0.2.
