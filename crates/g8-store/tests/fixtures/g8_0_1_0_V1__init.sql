-- ============================================================
-- g8-store schema v1
-- Applied via refinery on `g8 init` / first store open.
-- ARCHITECTURE.md §8 is the canonical DDL source.
-- ============================================================

CREATE TABLE IF NOT EXISTS convergence_space (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    root_path   TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    meta        TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS project (
    id          TEXT PRIMARY KEY,
    space_id    TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    root_path   TEXT NOT NULL,
    language    TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    meta        TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS capability (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    substrate       TEXT,
    consumes        TEXT NOT NULL DEFAULT '[]',
    produces        TEXT NOT NULL DEFAULT '[]',
    status          TEXT NOT NULL DEFAULT 'in_flight'
        CHECK (status IN ('proposed','in_flight','landed','stale','superseded','parked')),
    stub            INTEGER NOT NULL DEFAULT 0,
    stub_since      TEXT,
    owner_agent     TEXT,
    owner_team      TEXT,
    owner_contact   TEXT,
    file_path       TEXT,
    line_number     INTEGER,
    column_number   INTEGER,
    annotation_raw  TEXT,
    conditional     INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    meta            TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS intent (
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

CREATE TABLE IF NOT EXISTS plan (
    id              TEXT PRIMARY KEY,
    space_id        TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    project_id      TEXT          REFERENCES project(id) ON DELETE SET NULL,
    title           TEXT NOT NULL,
    description     TEXT,
    status          TEXT NOT NULL DEFAULT 'Idea'
        CHECK (status IN ('Idea','Scoped','Dispatched','Blocked','Done','Parked')),
    substrate       TEXT,
    wip_weight      INTEGER NOT NULL DEFAULT 1,
    parked_reason   TEXT,
    blocked_reason  TEXT,
    dispatched_at   INTEGER,
    completed_at    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    meta            TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS plan_capability (
    plan_id       TEXT NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    capability_id TEXT NOT NULL REFERENCES capability(id) ON DELETE CASCADE,
    overlap_kind  TEXT NOT NULL DEFAULT 'touches'
        CHECK (overlap_kind IN ('touches','extends','replaces','conflicts')),
    PRIMARY KEY (plan_id, capability_id)
) STRICT;

CREATE TABLE IF NOT EXISTS plan_intent (
    plan_id   TEXT NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    intent_id TEXT NOT NULL REFERENCES intent(id) ON DELETE CASCADE,
    relation  TEXT NOT NULL DEFAULT 'satisfies'
        CHECK (relation IN ('satisfies','contradicts','extends','derived_from')),
    PRIMARY KEY (plan_id, intent_id)
) STRICT;

CREATE TABLE IF NOT EXISTS plan_decision (
    plan_id     TEXT NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    decision_id TEXT NOT NULL REFERENCES decision(id) ON DELETE CASCADE,
    relation    TEXT NOT NULL DEFAULT 'governed_by'
        CHECK (relation IN ('governed_by','violates','supersedes')),
    PRIMARY KEY (plan_id, decision_id)
) STRICT;

CREATE TABLE IF NOT EXISTS decision (
    id          TEXT PRIMARY KEY,
    space_id    TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'proposed'
        CHECK (status IN ('proposed','accepted','rejected','superseded','ruled_out')),
    context     TEXT,
    body        TEXT NOT NULL DEFAULT '',
    source_file TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    meta        TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS substrate_budget (
    space_id              TEXT NOT NULL REFERENCES convergence_space(id) ON DELETE CASCADE,
    substrate             TEXT NOT NULL,
    wip_cap               INTEGER NOT NULL DEFAULT 3,
    stale_threshold_days  INTEGER NOT NULL DEFAULT 14,
    last_updated          INTEGER NOT NULL,
    PRIMARY KEY (space_id, substrate)
) STRICT;

CREATE TABLE IF NOT EXISTS capability_alias (
    canonical  TEXT NOT NULL,
    alias      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (canonical, alias)
) STRICT;

CREATE TABLE IF NOT EXISTS annotation (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    payload         TEXT NOT NULL,
    file_path       TEXT NOT NULL,
    line_number     INTEGER,
    column_number   INTEGER,
    source_lang     TEXT,
    source_kind     TEXT,
    conditional     INTEGER NOT NULL DEFAULT 0,
    extracted_at    INTEGER NOT NULL,
    meta            TEXT
) STRICT;

-- ============================================================
-- Indexes (ARCHITECTURE.md §8)
-- ============================================================

CREATE INDEX IF NOT EXISTS idx_plan_space_status        ON plan(space_id, status);
CREATE INDEX IF NOT EXISTS idx_plan_project             ON plan(project_id);
CREATE INDEX IF NOT EXISTS idx_plan_substrate           ON plan(substrate);
CREATE INDEX IF NOT EXISTS idx_capability_project_name  ON capability(project_id, name);
CREATE INDEX IF NOT EXISTS idx_capability_substrate     ON capability(substrate);
CREATE INDEX IF NOT EXISTS idx_capability_status        ON capability(status);
CREATE INDEX IF NOT EXISTS idx_intent_space_scope       ON intent(space_id, scope_path, scope_depth);
CREATE INDEX IF NOT EXISTS idx_intent_substrate         ON intent(substrate);
CREATE INDEX IF NOT EXISTS idx_intent_kind              ON intent(kind);
CREATE INDEX IF NOT EXISTS idx_annotation_project_kind  ON annotation(project_id, kind);
CREATE INDEX IF NOT EXISTS idx_plan_capability_cap      ON plan_capability(capability_id);
CREATE INDEX IF NOT EXISTS idx_plan_intent_intent       ON plan_intent(intent_id);
CREATE INDEX IF NOT EXISTS idx_capability_alias_alias   ON capability_alias(alias);
