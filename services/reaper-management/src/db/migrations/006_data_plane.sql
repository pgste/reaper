-- Data Plane: managed Authorization Data Model (ADM) per namespace.
-- See docs/development/DATA_PLANE_PLAN.md (Phase D1).

-- One datastore per (org, namespace): holds the model definition (schema
-- layer) and version counters. Records live in the adm_* tables below.
CREATE TABLE IF NOT EXISTS datastores (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    namespace_id TEXT NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    template TEXT NOT NULL,                  -- 'rbac' | 'abac' | 'rebac' | 'combined'
    model TEXT NOT NULL,                     -- JSON ModelDefinition (typed vocabulary)
    current_version INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, namespace_id)
);

CREATE INDEX IF NOT EXISTS idx_datastores_org ON datastores(org_id);

-- Entities: subjects/resources with TYPED attributes (validated against the
-- model at write time — the type-strict comparison contract enforced at the
-- source).
CREATE TABLE IF NOT EXISTS adm_entities (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    entity_id TEXT NOT NULL,                 -- the id policies see (e.g. "alice")
    entity_type TEXT NOT NULL,               -- must exist in model.entity_types
    attributes TEXT NOT NULL DEFAULT '{}',   -- JSON object, schema-validated
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(datastore_id, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_adm_entities_store ON adm_entities(datastore_id);
CREATE INDEX IF NOT EXISTS idx_adm_entities_type ON adm_entities(datastore_id, entity_type);

-- Role bindings: subject -> role (RBAC records — roles are model vocabulary).
-- scope '' means org/namespace-wide.
CREATE TABLE IF NOT EXISTS adm_role_bindings (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,                   -- entity_id of a user/group
    role TEXT NOT NULL,                      -- must exist in model.roles
    scope TEXT NOT NULL DEFAULT '',          -- optional resource/namespace scope
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, subject, role, scope)
);

CREATE INDEX IF NOT EXISTS idx_adm_bindings_store ON adm_role_bindings(datastore_id);
CREATE INDEX IF NOT EXISTS idx_adm_bindings_role ON adm_role_bindings(datastore_id, role);
CREATE INDEX IF NOT EXISTS idx_adm_bindings_subject ON adm_role_bindings(datastore_id, subject);

-- Relationship tuples: (object, relation, subject) — Zanzibar shape,
-- materialized verbatim into entity relationships{} for the ReBAC graph.
CREATE TABLE IF NOT EXISTS adm_tuples (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    object TEXT NOT NULL,                    -- entity_id the relation is ON
    relation TEXT NOT NULL,                  -- must exist in model.relations
    subject TEXT NOT NULL,                   -- entity_id that HOLDS the relation
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, object, relation, subject)
);

CREATE INDEX IF NOT EXISTS idx_adm_tuples_store ON adm_tuples(datastore_id);
CREATE INDEX IF NOT EXISTS idx_adm_tuples_object ON adm_tuples(datastore_id, object, relation);
CREATE INDEX IF NOT EXISTS idx_adm_tuples_subject ON adm_tuples(datastore_id, subject);

-- Published versions: immutable materialized data bundles. `document` is the
-- exact policy-engine DataLoader format — agents/sync fetch it by version.
CREATE TABLE IF NOT EXISTS adm_versions (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    checksum TEXT NOT NULL,                  -- sha256 of the document
    document TEXT NOT NULL,                  -- materialized entities JSON
    entity_count INTEGER NOT NULL,
    tuple_count INTEGER NOT NULL,
    binding_count INTEGER NOT NULL,
    published_by TEXT NOT NULL,
    published_at TEXT NOT NULL,
    UNIQUE(datastore_id, version)
);

CREATE INDEX IF NOT EXISTS idx_adm_versions_store ON adm_versions(datastore_id, version);
