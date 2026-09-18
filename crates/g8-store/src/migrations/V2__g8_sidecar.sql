-- ============================================================
-- g8-store schema v2: rename the `govern_sidecar` intent source
-- kind to `g8_sidecar`.
--
-- The govern-to-g8 rename edited V1__init.sql in place. refinery
-- checksums applied migrations, so every store initialised by
-- govern 0.1.0 refused to open under g8 ("applied migration V1 is
-- different than filesystem one"). V1 is restored byte-for-byte;
-- this migration carries the rename instead.
--
-- SQLite cannot alter a CHECK constraint, so `intent` is rebuilt
-- (create new, copy, drop old, rename, recreate indexes). Foreign
-- keys are switched off around the migration run in
-- RusqliteStore::migrate, because PRAGMA foreign_keys is a no-op
-- inside refinery's transaction, and PRAGMA foreign_key_check runs
-- once the run is over.
-- ============================================================

CREATE TABLE intent_v2 (
    id            TEXT PRIMARY KEY,
    space_id      TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    project_id    TEXT          REFERENCES project(id) ON DELETE SET NULL,
    kind          TEXT NOT NULL
        CHECK (kind IN ('architectural_scope','boundary','operational','tech_stack','explicit_intent','unclassified')),
    heading       TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    scope_path    TEXT NOT NULL,
    scope_depth   INTEGER NOT NULL,
    substrate     TEXT,
    owner_agent   TEXT,
    owner_team    TEXT,
    owner_contact TEXT,
    source_file   TEXT NOT NULL,
    source_line   INTEGER,
    source_kind   TEXT NOT NULL
        CHECK (source_kind IN ('agents_md','claude_md','g8_sidecar','inline_comment','manual')),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    meta          TEXT
) STRICT;

INSERT INTO intent_v2 (
    id, space_id, project_id, kind, heading, description, scope_path, scope_depth,
    substrate, owner_agent, owner_team, owner_contact, source_file, source_line,
    source_kind, created_at, updated_at, meta
)
SELECT
    id, space_id, project_id, kind, heading, description, scope_path, scope_depth,
    substrate, owner_agent, owner_team, owner_contact, source_file, source_line,
    CASE source_kind WHEN 'govern_sidecar' THEN 'g8_sidecar' ELSE source_kind END,
    created_at, updated_at, meta
FROM intent;

DROP TABLE intent;
ALTER TABLE intent_v2 RENAME TO intent;

CREATE INDEX IF NOT EXISTS idx_intent_space_scope       ON intent(space_id, scope_path, scope_depth);
CREATE INDEX IF NOT EXISTS idx_intent_substrate         ON intent(substrate);
CREATE INDEX IF NOT EXISTS idx_intent_kind              ON intent(kind);
