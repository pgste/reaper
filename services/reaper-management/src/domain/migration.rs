//! Data-model migration engine (Plan 12) — typed transforms over the ADM.
//!
//! Changing the *shape* of the authorization data model (renaming a role,
//! retyping an attribute, removing a relation) must never be a blind JSON
//! overwrite: every existing entity/binding/tuple record has a defined,
//! auditable transformation. This module is the closed, typed transform set
//! (ADR-2: no free-form scripts — a closed set keeps dry-run impact analysis
//! tractable and inverses mechanical) plus the in-memory planner that
//! computes exactly which records change and whether the migration is
//! applyable at all.
//!
//! The planner is deliberately pure (no I/O): it takes the current model and
//! full record sets and returns the transformed copies. Dry-run (Phase 1)
//! materializes those copies for impact analysis; atomic apply (Phase 2)
//! persists the same computed state — one code path decides what a
//! migration does.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::datastore::{
    AdmEntity, AttrType, AttributeDef, ModelDefinition, RelationDef, RelationTuple, RoleBinding,
};

// ---------------------------------------------------------------------------
// Transform set (closed, typed — ADR-2)
// ---------------------------------------------------------------------------

/// One typed model transform. Each variant declares its effect on the model
/// (`apply_to_model`), its effect on records (via the planner), and its
/// inverse (`inverse`), so rollback is a mechanical forward migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ModelTransform {
    /// Rename a role; every role binding follows. Decision-neutral: the
    /// role's permission set carries over under the new name.
    RenameRole { from: String, to: String },
    /// Rename a relation; every tuple follows. Decision-neutral at the data
    /// level (edges are isomorphic under the rename); policies referencing
    /// the old name must be updated in the same release.
    RenameRelation { from: String, to: String },
    /// Rename an attribute on one entity type; every stored value follows.
    RenameAttribute {
        entity_type: String,
        from: String,
        to: String,
    },
    /// Rename an entity type; entity records and the relation definitions
    /// referencing the type (object/subject) follow.
    RenameEntityType { from: String, to: String },
    /// Add an attribute definition (schema-only; no records change).
    AddAttribute {
        entity_type: String,
        def: AttributeDef,
    },
    /// Remove an attribute definition AND strip the stored values.
    /// Irreversible (`inverse() = None`): the values are gone — rollback
    /// requires an explicit backfill (the immutable pre-migration
    /// `adm_versions` document is the safety net).
    RemoveAttribute { entity_type: String, name: String },
    /// Add a relation definition (schema-only; no records change).
    AddRelation { def: RelationDef },
    /// Remove a relation definition. Refuses to plan while tuples exist
    /// unless `delete_tuples` is set — deleting live edges is an access
    /// change and must be asked for explicitly.
    RemoveRelation {
        name: String,
        #[serde(default)]
        delete_tuples: bool,
    },
    /// Change an attribute's type, coercing every stored value. A value
    /// that cannot be coerced fails the plan closed (ADR-4) unless an
    /// explicit `default` is supplied to take its place.
    RetypeAttribute {
        entity_type: String,
        name: String,
        to: AttrType,
        /// Used when a stored value cannot be coerced. Must itself be valid
        /// for the target type. None = un-coercible values block the plan.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<serde_json::Value>,
    },
}

impl ModelTransform {
    /// Apply this transform to the model, validating preconditions
    /// (source exists, target name free, referenced types known).
    pub fn apply_to_model(&self, model: &ModelDefinition) -> Result<ModelDefinition, String> {
        let mut m = model.clone();
        match self {
            Self::RenameRole { from, to } => {
                if m.role(to).is_some() {
                    return Err(format!("role '{to}' already exists"));
                }
                let role = m
                    .roles
                    .iter_mut()
                    .find(|r| &r.name == from)
                    .ok_or_else(|| format!("role '{from}' not found in model"))?;
                role.name = to.clone();
            }
            Self::RenameRelation { from, to } => {
                if m.relation(to).is_some() {
                    return Err(format!("relation '{to}' already exists"));
                }
                let rel = m
                    .relations
                    .iter_mut()
                    .find(|r| &r.name == from)
                    .ok_or_else(|| format!("relation '{from}' not found in model"))?;
                rel.name = to.clone();
            }
            Self::RenameAttribute {
                entity_type,
                from,
                to,
            } => {
                let t = m
                    .entity_types
                    .iter_mut()
                    .find(|t| &t.name == entity_type)
                    .ok_or_else(|| format!("entity type '{entity_type}' not found"))?;
                if t.attributes.iter().any(|a| &a.name == to) {
                    return Err(format!(
                        "attribute '{to}' already exists on '{entity_type}'"
                    ));
                }
                let attr = t
                    .attributes
                    .iter_mut()
                    .find(|a| &a.name == from)
                    .ok_or_else(|| {
                        format!("attribute '{from}' not found on entity type '{entity_type}'")
                    })?;
                attr.name = to.clone();
            }
            Self::RenameEntityType { from, to } => {
                if m.entity_type(to).is_some() {
                    return Err(format!("entity type '{to}' already exists"));
                }
                let t = m
                    .entity_types
                    .iter_mut()
                    .find(|t| &t.name == from)
                    .ok_or_else(|| format!("entity type '{from}' not found"))?;
                t.name = to.clone();
                // Relation definitions reference entity types — follow them.
                for rel in &mut m.relations {
                    if &rel.object == from {
                        rel.object = to.clone();
                    }
                    for s in &mut rel.subject {
                        if s == from {
                            *s = to.clone();
                        }
                    }
                }
            }
            Self::AddAttribute { entity_type, def } => {
                let t = m
                    .entity_types
                    .iter_mut()
                    .find(|t| &t.name == entity_type)
                    .ok_or_else(|| format!("entity type '{entity_type}' not found"))?;
                if t.attributes.iter().any(|a| a.name == def.name) {
                    return Err(format!(
                        "attribute '{}' already exists on '{entity_type}'",
                        def.name
                    ));
                }
                t.attributes.push(def.clone());
            }
            Self::RemoveAttribute { entity_type, name } => {
                let t = m
                    .entity_types
                    .iter_mut()
                    .find(|t| &t.name == entity_type)
                    .ok_or_else(|| format!("entity type '{entity_type}' not found"))?;
                let before = t.attributes.len();
                t.attributes.retain(|a| &a.name != name);
                if t.attributes.len() == before {
                    return Err(format!(
                        "attribute '{name}' not found on entity type '{entity_type}'"
                    ));
                }
            }
            Self::AddRelation { def } => {
                if m.relation(&def.name).is_some() {
                    return Err(format!("relation '{}' already exists", def.name));
                }
                if m.entity_type(&def.object).is_none() {
                    return Err(format!(
                        "relation object type '{}' not found in model",
                        def.object
                    ));
                }
                for s in &def.subject {
                    if m.entity_type(s).is_none() {
                        return Err(format!("relation subject type '{s}' not found in model"));
                    }
                }
                m.relations.push(def.clone());
            }
            Self::RemoveRelation { name, .. } => {
                let before = m.relations.len();
                m.relations.retain(|r| &r.name != name);
                if m.relations.len() == before {
                    return Err(format!("relation '{name}' not found in model"));
                }
            }
            Self::RetypeAttribute {
                entity_type,
                name,
                to,
                default,
            } => {
                let t = m
                    .entity_types
                    .iter_mut()
                    .find(|t| &t.name == entity_type)
                    .ok_or_else(|| format!("entity type '{entity_type}' not found"))?;
                let attr = t
                    .attributes
                    .iter_mut()
                    .find(|a| &a.name == name)
                    .ok_or_else(|| {
                        format!("attribute '{name}' not found on entity type '{entity_type}'")
                    })?;
                if attr.attr_type == *to {
                    return Err(format!("attribute '{name}' is already {to:?}"));
                }
                if let Some(d) = default {
                    if coerce_value(d, *to).is_err() {
                        return Err(format!(
                            "retype default {d} is not itself a valid {to:?} value"
                        ));
                    }
                }
                attr.attr_type = *to;
                // A closed value set is a set of STRINGS — it cannot survive
                // a retype away from String.
                if *to != AttrType::String {
                    attr.values = None;
                }
            }
        }
        Ok(m)
    }

    /// The inverse transform, computed against the model the forward
    /// transform was applied TO (some inverses need the removed definition
    /// back). `None` = irreversible: rollback requires an explicit backfill.
    pub fn inverse(&self, model_before: &ModelDefinition) -> Option<ModelTransform> {
        match self {
            Self::RenameRole { from, to } => Some(Self::RenameRole {
                from: to.clone(),
                to: from.clone(),
            }),
            Self::RenameRelation { from, to } => Some(Self::RenameRelation {
                from: to.clone(),
                to: from.clone(),
            }),
            Self::RenameAttribute {
                entity_type,
                from,
                to,
            } => Some(Self::RenameAttribute {
                entity_type: entity_type.clone(),
                from: to.clone(),
                to: from.clone(),
            }),
            Self::RenameEntityType { from, to } => Some(Self::RenameEntityType {
                from: to.clone(),
                to: from.clone(),
            }),
            Self::AddAttribute { entity_type, def } => Some(Self::RemoveAttribute {
                entity_type: entity_type.clone(),
                name: def.name.clone(),
            }),
            // The stripped values are unrecoverable from the model alone.
            Self::RemoveAttribute { .. } => None,
            Self::AddRelation { def } => Some(Self::RemoveRelation {
                name: def.name.clone(),
                delete_tuples: true,
            }),
            Self::RemoveRelation { name, .. } => {
                // Restore the definition captured from the pre-model. The
                // deleted TUPLES are not restored by the inverse — like
                // RemoveAttribute's values, record data needs the immutable
                // pre-migration version document.
                model_before
                    .relation(name)
                    .map(|def| Self::AddRelation { def: def.clone() })
            }
            Self::RetypeAttribute {
                entity_type, name, ..
            } => {
                let from_type = model_before
                    .entity_type(entity_type)?
                    .attributes
                    .iter()
                    .find(|a| &a.name == name)?
                    .attr_type;
                Some(Self::RetypeAttribute {
                    entity_type: entity_type.clone(),
                    name: name.clone(),
                    to: from_type,
                    default: None,
                })
            }
        }
    }
}

/// Coerce one stored value to a target attribute type. Lossless coercions
/// only — anything else is an error the caller must resolve explicitly
/// (ADR-4: a silent coercion in an authorization datastore is an access
/// incident).
pub fn coerce_value(value: &serde_json::Value, to: AttrType) -> Result<serde_json::Value, String> {
    use serde_json::Value as V;
    let ok = |v: V| Ok(v);
    match (value, to) {
        (V::String(_), AttrType::String)
        | (V::Bool(_), AttrType::Bool)
        | (V::Number(_), AttrType::Int)
        | (V::Array(_), AttrType::StringList) => {
            // Same shape — still validate the tricky ones.
            match to {
                AttrType::Int if value.as_i64().is_none() && value.as_u64().is_none() => {
                    Err(format!("{value} is not an integer"))
                }
                AttrType::StringList
                    if !value
                        .as_array()
                        .is_some_and(|a| a.iter().all(|v| v.is_string())) =>
                {
                    Err(format!("{value} is not a list of strings"))
                }
                _ => ok(value.clone()),
            }
        }
        (V::Number(n), AttrType::String) => ok(V::String(n.to_string())),
        (V::Bool(b), AttrType::String) => ok(V::String(b.to_string())),
        (V::String(s), AttrType::Int) => s
            .trim()
            .parse::<i64>()
            .map(|i| V::Number(i.into()))
            .map_err(|_| format!("{s:?} is not parseable as an integer")),
        (V::String(s), AttrType::Bool) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => ok(V::Bool(true)),
            "false" => ok(V::Bool(false)),
            _ => Err(format!("{s:?} is not parseable as a boolean")),
        },
        (V::String(s), AttrType::StringList) => ok(V::Array(vec![V::String(s.clone())])),
        (v, t) => Err(format!("cannot coerce {v} to {t:?}")),
    }
}

// ---------------------------------------------------------------------------
// Planner: record-level effect of a transform chain
// ---------------------------------------------------------------------------

/// One record-level operation the migration performs, with its exact
/// affected-row count — the human-reviewable half of the dry-run.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordOp {
    UpdateBindingRole {
        from: String,
        to: String,
        bindings: usize,
    },
    UpdateTupleRelation {
        from: String,
        to: String,
        tuples: usize,
    },
    RenameEntityAttribute {
        entity_type: String,
        from: String,
        to: String,
        entities: usize,
    },
    UpdateEntityType {
        from: String,
        to: String,
        entities: usize,
    },
    RemoveEntityAttribute {
        entity_type: String,
        name: String,
        entities: usize,
    },
    DeleteTuples {
        relation: String,
        tuples: usize,
    },
    RetypeEntityAttribute {
        entity_type: String,
        name: String,
        to: AttrType,
        coerced: usize,
        defaulted: usize,
    },
}

/// A reason the plan cannot be applied as-is (fails closed, ADR-4).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanBlocker {
    /// A stored value cannot be coerced to the retype target and no
    /// `default` was supplied.
    Coercion {
        entity_type: String,
        attribute: String,
        /// Offending entity ids (capped — `total` is authoritative).
        entity_ids: Vec<String>,
        total: usize,
    },
    /// RemoveRelation over live tuples without `delete_tuples: true`.
    RelationNotEmpty { relation: String, tuples: usize },
}

/// The full dry-run product: transformed model + records, the per-op counts,
/// and anything blocking apply.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub transforms: Vec<ModelTransform>,
    pub model_before: ModelDefinition,
    pub model_after: ModelDefinition,
    pub entities_after: Vec<AdmEntity>,
    pub bindings_after: Vec<RoleBinding>,
    pub tuples_after: Vec<RelationTuple>,
    pub record_ops: Vec<RecordOp>,
    pub blockers: Vec<PlanBlocker>,
}

impl MigrationPlan {
    pub fn applyable(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// How many offending entity ids a blocker lists before switching to a
/// count-only summary — the report must stay readable at 100k records.
const BLOCKER_ID_CAP: usize = 20;

/// Compute the full plan: transforms applied in order to the model AND to
/// in-memory copies of every record. Pure — nothing is persisted; the
/// returned record sets are exactly what apply (Phase 2) will write.
pub fn plan(
    transforms: &[ModelTransform],
    model: &ModelDefinition,
    entities: &[AdmEntity],
    bindings: &[RoleBinding],
    tuples: &[RelationTuple],
) -> Result<MigrationPlan, String> {
    if transforms.is_empty() {
        return Err("a migration needs at least one transform".to_string());
    }

    let mut m = model.clone();
    let mut ents = entities.to_vec();
    let mut binds = bindings.to_vec();
    let mut tups = tuples.to_vec();
    let mut record_ops = Vec::new();
    let mut blockers = Vec::new();

    for t in transforms {
        // Model first — it validates the transform's preconditions.
        let next_model = t.apply_to_model(&m)?;

        match t {
            ModelTransform::RenameRole { from, to } => {
                let mut n = 0;
                for b in binds.iter_mut().filter(|b| &b.role == from) {
                    b.role = to.clone();
                    n += 1;
                }
                record_ops.push(RecordOp::UpdateBindingRole {
                    from: from.clone(),
                    to: to.clone(),
                    bindings: n,
                });
            }
            ModelTransform::RenameRelation { from, to } => {
                let mut n = 0;
                for tup in tups.iter_mut().filter(|t| &t.relation == from) {
                    tup.relation = to.clone();
                    n += 1;
                }
                record_ops.push(RecordOp::UpdateTupleRelation {
                    from: from.clone(),
                    to: to.clone(),
                    tuples: n,
                });
            }
            ModelTransform::RenameAttribute {
                entity_type,
                from,
                to,
            } => {
                let mut n = 0;
                for e in ents.iter_mut().filter(|e| &e.entity_type == entity_type) {
                    if let Some(v) = e.attributes.remove(from) {
                        e.attributes.insert(to.clone(), v);
                        n += 1;
                    }
                }
                record_ops.push(RecordOp::RenameEntityAttribute {
                    entity_type: entity_type.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    entities: n,
                });
            }
            ModelTransform::RenameEntityType { from, to } => {
                let mut n = 0;
                for e in ents.iter_mut().filter(|e| &e.entity_type == from) {
                    e.entity_type = to.clone();
                    n += 1;
                }
                record_ops.push(RecordOp::UpdateEntityType {
                    from: from.clone(),
                    to: to.clone(),
                    entities: n,
                });
            }
            ModelTransform::AddAttribute { .. } | ModelTransform::AddRelation { .. } => {
                // Schema-only.
            }
            ModelTransform::RemoveAttribute { entity_type, name } => {
                let mut n = 0;
                for e in ents.iter_mut().filter(|e| &e.entity_type == entity_type) {
                    if e.attributes.remove(name).is_some() {
                        n += 1;
                    }
                }
                record_ops.push(RecordOp::RemoveEntityAttribute {
                    entity_type: entity_type.clone(),
                    name: name.clone(),
                    entities: n,
                });
            }
            ModelTransform::RemoveRelation {
                name,
                delete_tuples,
            } => {
                let matching = tups.iter().filter(|t| &t.relation == name).count();
                if matching > 0 && !delete_tuples {
                    blockers.push(PlanBlocker::RelationNotEmpty {
                        relation: name.clone(),
                        tuples: matching,
                    });
                } else if matching > 0 {
                    tups.retain(|t| &t.relation != name);
                    record_ops.push(RecordOp::DeleteTuples {
                        relation: name.clone(),
                        tuples: matching,
                    });
                }
            }
            ModelTransform::RetypeAttribute {
                entity_type,
                name,
                to,
                default,
            } => {
                let mut coerced = 0;
                let mut defaulted = 0;
                let mut failed: Vec<String> = Vec::new();
                for e in ents.iter_mut().filter(|e| &e.entity_type == entity_type) {
                    let Some(v) = e.attributes.get(name) else {
                        continue;
                    };
                    match coerce_value(v, *to) {
                        Ok(new_v) => {
                            e.attributes.insert(name.clone(), new_v);
                            coerced += 1;
                        }
                        Err(_) => match default {
                            Some(d) => {
                                e.attributes.insert(name.clone(), d.clone());
                                defaulted += 1;
                            }
                            None => failed.push(e.entity_id.clone()),
                        },
                    }
                }
                if failed.is_empty() {
                    record_ops.push(RecordOp::RetypeEntityAttribute {
                        entity_type: entity_type.clone(),
                        name: name.clone(),
                        to: *to,
                        coerced,
                        defaulted,
                    });
                } else {
                    let total = failed.len();
                    failed.truncate(BLOCKER_ID_CAP);
                    blockers.push(PlanBlocker::Coercion {
                        entity_type: entity_type.clone(),
                        attribute: name.clone(),
                        entity_ids: failed,
                        total,
                    });
                }
            }
        }

        m = next_model;
    }

    Ok(MigrationPlan {
        transforms: transforms.to_vec(),
        model_before: model.clone(),
        model_after: m,
        entities_after: ents,
        bindings_after: binds,
        tuples_after: tups,
        record_ops,
        blockers,
    })
}

/// The vocabulary rename maps a transform chain implies — used by impact
/// analysis to compare BEFORE access expressed in AFTER vocabulary, so a
/// pure rename diffs to zero instead of "everything removed + re-added".
#[derive(Debug, Default, Clone)]
pub struct RenameMaps {
    pub relations: BTreeMap<String, String>,
    pub roles: BTreeMap<String, String>,
}

/// Compose the rollback of an applied migration as a NEW forward transform
/// chain (ADR-3: history is append-only; an undo is a visible event, never
/// a rewritten past). Inverses are computed against the model state each
/// forward transform was applied to (replayed from `model_before`) and
/// emitted in REVERSE order. Errors if any transform is irreversible
/// (RemoveAttribute — its values need an explicit backfill) or the replay
/// fails.
pub fn compose_rollback(
    transforms: &[ModelTransform],
    model_before: &ModelDefinition,
) -> Result<Vec<ModelTransform>, String> {
    let mut state = model_before.clone();
    let mut inverses = Vec::with_capacity(transforms.len());
    for t in transforms {
        let inv = t.inverse(&state).ok_or_else(|| {
            format!(
                "transform {t:?} is irreversible (its record data is gone); \
                 restore from the pre-migration data version instead"
            )
        })?;
        state = t.apply_to_model(&state)?;
        inverses.push(inv);
    }
    inverses.reverse();
    Ok(inverses)
}

/// The vocabulary-breaking changes a bare model overwrite would make —
/// used by `PUT …/model` to REJECT silent renames/removals/retypes and
/// direct callers to the migration endpoint. Additive edits (new roles,
/// relations, types, attributes) and permission-list tuning pass; anything
/// that would strand existing records does not.
pub fn vocabulary_breaking_changes(old: &ModelDefinition, new: &ModelDefinition) -> Vec<String> {
    let mut breaks = Vec::new();
    for role in &old.roles {
        if new.role(&role.name).is_none() {
            breaks.push(format!("role '{}' removed", role.name));
        }
    }
    for rel in &old.relations {
        match new.relation(&rel.name) {
            None => breaks.push(format!("relation '{}' removed", rel.name)),
            Some(n) if n.traversal != rel.traversal => breaks.push(format!(
                "relation '{}' traversal changed (flips edge direction)",
                rel.name
            )),
            Some(n) if n.object != rel.object => breaks.push(format!(
                "relation '{}' object type changed ('{}' → '{}')",
                rel.name, rel.object, n.object
            )),
            _ => {}
        }
    }
    for t in &old.entity_types {
        let Some(nt) = new.entity_type(&t.name) else {
            breaks.push(format!("entity type '{}' removed", t.name));
            continue;
        };
        for attr in &t.attributes {
            match nt.attributes.iter().find(|a| a.name == attr.name) {
                None => breaks.push(format!("attribute '{}.{}' removed", t.name, attr.name)),
                Some(na) if na.attr_type != attr.attr_type => breaks.push(format!(
                    "attribute '{}.{}' retyped ({:?} → {:?})",
                    t.name, attr.name, attr.attr_type, na.attr_type
                )),
                _ => {}
            }
        }
    }
    breaks
}

pub fn rename_maps(transforms: &[ModelTransform]) -> RenameMaps {
    let mut maps = RenameMaps::default();
    for t in transforms {
        match t {
            ModelTransform::RenameRelation { from, to } => {
                // Follow chains: a→b then b→c must map a→c.
                let source = maps
                    .relations
                    .iter()
                    .find(|(_, v)| *v == from)
                    .map(|(k, _)| k.clone());
                match source {
                    Some(k) => {
                        maps.relations.insert(k, to.clone());
                    }
                    None => {
                        maps.relations.insert(from.clone(), to.clone());
                    }
                }
            }
            ModelTransform::RenameRole { from, to } => {
                let source = maps
                    .roles
                    .iter()
                    .find(|(_, v)| *v == from)
                    .map(|(k, _)| k.clone());
                match source {
                    Some(k) => {
                        maps.roles.insert(k, to.clone());
                    }
                    None => {
                        maps.roles.insert(from.clone(), to.clone());
                    }
                }
            }
            _ => {}
        }
    }
    maps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::datastore::DatastoreTemplate;

    fn model() -> ModelDefinition {
        DatastoreTemplate::Combined.seed_model()
    }

    fn entity(id: &str, ty: &str, attrs: serde_json::Value) -> AdmEntity {
        AdmEntity {
            entity_id: id.into(),
            entity_type: ty.into(),
            attributes: attrs.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn rename_role_moves_model_and_bindings() {
        let m = model();
        let t = ModelTransform::RenameRole {
            from: "editor".into(),
            to: "author".into(),
        };
        let m2 = t.apply_to_model(&m).unwrap();
        assert!(m2.role("editor").is_none());
        assert_eq!(
            m2.role("author").unwrap().permissions,
            m.role("editor").unwrap().permissions,
            "permissions carry over — the rename is decision-neutral"
        );

        let bindings = vec![
            RoleBinding {
                subject: "alice".into(),
                role: "editor".into(),
                scope: String::new(),
            },
            RoleBinding {
                subject: "bob".into(),
                role: "viewer".into(),
                scope: String::new(),
            },
        ];
        let plan = plan(&[t], &m, &[], &bindings, &[]).unwrap();
        assert!(plan.applyable());
        assert_eq!(plan.bindings_after[0].role, "author");
        assert_eq!(plan.bindings_after[1].role, "viewer");
        assert!(matches!(
            plan.record_ops[0],
            RecordOp::UpdateBindingRole { bindings: 1, .. }
        ));
    }

    #[test]
    fn inverse_round_trips_reversible_transforms() {
        let m = model();
        let reversible = vec![
            ModelTransform::RenameRole {
                from: "editor".into(),
                to: "author".into(),
            },
            ModelTransform::RenameRelation {
                from: "owner".into(),
                to: "possessor".into(),
            },
            ModelTransform::RenameAttribute {
                entity_type: "user".into(),
                from: "department".into(),
                to: "org_unit".into(),
            },
            ModelTransform::RenameEntityType {
                from: "group".into(),
                to: "team".into(),
            },
            ModelTransform::RetypeAttribute {
                entity_type: "user".into(),
                name: "clearance".into(),
                to: AttrType::String,
                default: None,
            },
            ModelTransform::RemoveRelation {
                name: "viewer".into(),
                delete_tuples: true,
            },
        ];
        // The model is semantically order-independent (lookups are by name);
        // an inverse may re-append a definition, so compare canonically.
        let canonical = |m: &ModelDefinition| -> serde_json::Value {
            let mut m = m.clone();
            m.entity_types.sort_by(|a, b| a.name.cmp(&b.name));
            m.roles.sort_by(|a, b| a.name.cmp(&b.name));
            m.relations.sort_by(|a, b| a.name.cmp(&b.name));
            serde_json::to_value(&m).unwrap()
        };
        for t in reversible {
            let after = t.apply_to_model(&m).unwrap();
            let inv = t
                .inverse(&m)
                .unwrap_or_else(|| panic!("{t:?} should be reversible"));
            let back = inv.apply_to_model(&after).unwrap();
            assert_eq!(
                canonical(&back),
                canonical(&m),
                "apply(inverse(apply(m))) == m for {t:?}"
            );
        }
    }

    #[test]
    fn remove_attribute_is_irreversible() {
        let m = model();
        let t = ModelTransform::RemoveAttribute {
            entity_type: "user".into(),
            name: "tags".into(),
        };
        assert!(t.inverse(&m).is_none(), "value loss cannot be inverted");
    }

    #[test]
    fn retype_fails_closed_on_uncoercible_value() {
        let m = model();
        // department is String; retype to Int with a non-numeric value.
        let ents = vec![
            entity("alice", "user", serde_json::json!({"department": "42"})),
            entity(
                "bob",
                "user",
                serde_json::json!({"department": "engineering"}),
            ),
        ];
        let t = ModelTransform::RetypeAttribute {
            entity_type: "user".into(),
            name: "department".into(),
            to: AttrType::Int,
            default: None,
        };
        let p = plan(&[t], &m, &ents, &[], &[]).unwrap();
        assert!(!p.applyable());
        match &p.blockers[0] {
            PlanBlocker::Coercion {
                entity_ids, total, ..
            } => {
                assert_eq!(*total, 1);
                assert_eq!(entity_ids, &vec!["bob".to_string()]);
            }
            other => panic!("expected coercion blocker, got {other:?}"),
        }

        // Same retype with an explicit default is applyable and counts both.
        let t = ModelTransform::RetypeAttribute {
            entity_type: "user".into(),
            name: "department".into(),
            to: AttrType::Int,
            default: Some(serde_json::json!(0)),
        };
        let p = plan(&[t], &m, &ents, &[], &[]).unwrap();
        assert!(p.applyable());
        assert!(matches!(
            p.record_ops[0],
            RecordOp::RetypeEntityAttribute {
                coerced: 1,
                defaulted: 1,
                ..
            }
        ));
        assert_eq!(p.entities_after[0].attributes["department"], 42);
        assert_eq!(p.entities_after[1].attributes["department"], 0);
    }

    #[test]
    fn remove_relation_refuses_live_tuples_without_flag() {
        let m = model();
        let tuples = vec![RelationTuple {
            object: "doc-1".into(),
            relation: "owner".into(),
            subject: "alice".into(),
        }];
        let t = ModelTransform::RemoveRelation {
            name: "owner".into(),
            delete_tuples: false,
        };
        let p = plan(&[t], &m, &[], &[], &tuples).unwrap();
        assert!(!p.applyable());
        assert!(matches!(
            p.blockers[0],
            PlanBlocker::RelationNotEmpty { tuples: 1, .. }
        ));

        let t = ModelTransform::RemoveRelation {
            name: "owner".into(),
            delete_tuples: true,
        };
        let p = plan(&[t], &m, &[], &[], &tuples).unwrap();
        assert!(p.applyable());
        assert!(p.tuples_after.is_empty());
    }

    #[test]
    fn rename_entity_type_follows_relation_defs_and_records() {
        let m = model();
        let ents = vec![entity("g1", "group", serde_json::json!({}))];
        let t = ModelTransform::RenameEntityType {
            from: "group".into(),
            to: "team".into(),
        };
        let p = plan(&[t], &m, &ents, &[], &[]).unwrap();
        assert_eq!(p.entities_after[0].entity_type, "team");
        let member_of = p.model_after.relation("member_of").unwrap();
        assert_eq!(member_of.object, "team");
        assert!(member_of.subject.contains(&"team".to_string()));
    }

    #[test]
    fn transform_chain_applies_in_order_and_rename_maps_follow() {
        let m = model();
        let ts = vec![
            ModelTransform::RenameRelation {
                from: "owner".into(),
                to: "holder".into(),
            },
            ModelTransform::RenameRelation {
                from: "holder".into(),
                to: "possessor".into(),
            },
        ];
        let p = plan(
            &ts,
            &m,
            &[],
            &[],
            &[RelationTuple {
                object: "d".into(),
                relation: "owner".into(),
                subject: "a".into(),
            }],
        )
        .unwrap();
        assert_eq!(p.tuples_after[0].relation, "possessor");
        let maps = rename_maps(&ts);
        assert_eq!(maps.relations.get("owner").unwrap(), "possessor");
    }

    #[test]
    fn compose_rollback_reverses_the_chain_and_refuses_irreversible() {
        let m = model();
        let ts = vec![
            ModelTransform::RenameRole {
                from: "editor".into(),
                to: "author".into(),
            },
            ModelTransform::RenameRole {
                from: "author".into(),
                to: "writer".into(),
            },
        ];
        let inv = compose_rollback(&ts, &m).unwrap();
        // Reverse order: undo writer->author first, then author->editor.
        assert_eq!(
            inv,
            vec![
                ModelTransform::RenameRole {
                    from: "writer".into(),
                    to: "author".into(),
                },
                ModelTransform::RenameRole {
                    from: "author".into(),
                    to: "editor".into(),
                },
            ]
        );
        // Applying forward then rollback lands on the original model.
        let mut state = m.clone();
        for t in ts.iter().chain(inv.iter()) {
            state = t.apply_to_model(&state).unwrap();
        }
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::to_value(&m).unwrap()
        );

        let irreversible = vec![ModelTransform::RemoveAttribute {
            entity_type: "user".into(),
            name: "tags".into(),
        }];
        let err = compose_rollback(&irreversible, &m).unwrap_err();
        assert!(err.contains("irreversible"), "{err}");
    }

    #[test]
    fn coercions_are_lossless_only() {
        use serde_json::json;
        assert_eq!(
            coerce_value(&json!(5), AttrType::String).unwrap(),
            json!("5")
        );
        assert_eq!(coerce_value(&json!("7"), AttrType::Int).unwrap(), json!(7));
        assert_eq!(
            coerce_value(&json!("true"), AttrType::Bool).unwrap(),
            json!(true)
        );
        assert_eq!(
            coerce_value(&json!("a"), AttrType::StringList).unwrap(),
            json!(["a"])
        );
        assert!(coerce_value(&json!("high"), AttrType::Int).is_err());
        assert!(coerce_value(&json!([1, 2]), AttrType::StringList).is_err());
        assert!(coerce_value(&json!(1), AttrType::Bool).is_err());
    }
}
