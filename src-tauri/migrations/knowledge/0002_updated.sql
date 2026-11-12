-- Knowledge store version 2 (Slice 3.8, ADR 0019): when a trail and a step
-- last changed, so an import can keep the newer of two copies. A rename, a
-- pin and a step's note left no time behind before. Existing rows start
-- from the times they already have.

ALTER TABLE trails ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE trails SET updated_at = last_step_at;

ALTER TABLE trail_steps ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE trail_steps SET updated_at = at;
