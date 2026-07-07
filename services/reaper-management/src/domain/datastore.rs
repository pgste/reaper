//! Authorization Data Model (ADM) — the data plane's schema layer.
//!
//! A namespace's datastore declares a typed vocabulary (entity types +
//! attributes, roles + permissions, relations) and holds records (entities,
//! role bindings, relationship tuples). `materialize` compiles everything to
//! the exact policy-engine DataLoader format, so reapers consume it with
//! zero engine changes. See docs/development/DATA_PLANE_PLAN.md.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Model definition (schema layer)
// ---------------------------------------------------------------------------

/// Attribute types match the engine's type-strict comparison contract —
/// validation HERE is what makes `"5"` vs `5` impossible at evaluation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttrType {
    String,
    Int,
    Bool,
    /// List of strings (tags, group names, scopes…)
    StringList,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeDef {
    pub name: String,
    #[serde(rename = "type")]
    pub attr_type: AttrType,
    /// Optional closed set of allowed values (strings only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityTypeDef {
    pub name: String,
    #[serde(default)]
    pub attributes: Vec<AttributeDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDef {
    pub name: String,
    /// Permission strings, e.g. "document:read" or "*:*".
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationDef {
    pub name: String,
    /// Entity type the relation is declared ON (the tuple's object).
    pub object: String,
    /// Entity types allowed as tuple subjects.
    #[serde(default)]
    pub subject: Vec<String>,
    /// TRAVERSAL relations (used as `via`/`up` in rebac::reachable/
    /// inherited) materialize the edge on the SUBJECT (subject → object),
    /// because the engine's BFS walks outward from the subject. Zanzibar
    /// tuples still read naturally — (group, member_of, alice) — but the
    /// graph edge lands as alice → group. Non-traversal relations (owner,
    /// viewer) stay on the object, which is what rebac::related reads.
    #[serde(default)]
    pub traversal: bool,
}

/// The full model definition for a datastore.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelDefinition {
    #[serde(default)]
    pub entity_types: Vec<EntityTypeDef>,
    #[serde(default)]
    pub roles: Vec<RoleDef>,
    #[serde(default)]
    pub relations: Vec<RelationDef>,
}

impl ModelDefinition {
    pub fn entity_type(&self, name: &str) -> Option<&EntityTypeDef> {
        self.entity_types.iter().find(|t| t.name == name)
    }
    pub fn role(&self, name: &str) -> Option<&RoleDef> {
        self.roles.iter().find(|r| r.name == name)
    }
    pub fn relation(&self, name: &str) -> Option<&RelationDef> {
        self.relations.iter().find(|r| r.name == name)
    }

    /// Validate an attribute map for `entity_type` against the schema.
    /// Unknown attributes and type mismatches are rejected — fail closed at
    /// WRITE time instead of silently-deny at evaluation time.
    pub fn validate_attributes(
        &self,
        entity_type: &str,
        attributes: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let type_def = self
            .entity_type(entity_type)
            .ok_or_else(|| format!("unknown entity type '{entity_type}' (not in model)"))?;

        for (key, value) in attributes {
            let def = type_def
                .attributes
                .iter()
                .find(|a| &a.name == key)
                .ok_or_else(|| {
                    format!("unknown attribute '{key}' for entity type '{entity_type}'")
                })?;

            let ok = match def.attr_type {
                AttrType::String => value.is_string(),
                AttrType::Int => value.is_i64() || value.is_u64(),
                AttrType::Bool => value.is_boolean(),
                AttrType::StringList => {
                    value.is_array()
                        && value
                            .as_array()
                            .is_some_and(|a| a.iter().all(|v| v.is_string()))
                }
            };
            if !ok {
                return Err(format!(
                    "attribute '{key}' must be {:?} (got {value}) — the engine's \
                     type-strict comparisons would never match a mistyped value",
                    def.attr_type
                ));
            }

            if let (Some(allowed), Some(s)) = (&def.values, value.as_str()) {
                if !allowed.iter().any(|v| v == s) {
                    return Err(format!(
                        "attribute '{key}' value {s:?} not in allowed set {allowed:?}"
                    ));
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatastoreTemplate {
    Rbac,
    Abac,
    Rebac,
    Combined,
}

impl DatastoreTemplate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rbac => "rbac",
            Self::Abac => "abac",
            Self::Rebac => "rebac",
            Self::Combined => "combined",
        }
    }

    /// Seed model for the template — opinionated starting vocabulary the UI
    /// managers open ready-to-edit.
    pub fn seed_model(&self) -> ModelDefinition {
        let user = |attrs: Vec<AttributeDef>| EntityTypeDef {
            name: "user".into(),
            attributes: attrs,
        };
        let resource = |attrs: Vec<AttributeDef>| EntityTypeDef {
            name: "resource".into(),
            attributes: attrs,
        };
        let group = EntityTypeDef {
            name: "group".into(),
            attributes: vec![],
        };
        let attr = |name: &str, t: AttrType| AttributeDef {
            name: name.into(),
            attr_type: t,
            values: None,
        };

        let rbac_roles = vec![
            RoleDef {
                name: "admin".into(),
                permissions: vec!["*:*".into()],
            },
            RoleDef {
                name: "editor".into(),
                permissions: vec!["resource:read".into(), "resource:write".into()],
            },
            RoleDef {
                name: "viewer".into(),
                permissions: vec!["resource:read".into()],
            },
        ];
        let rebac_relations = vec![
            RelationDef {
                name: "owner".into(),
                object: "resource".into(),
                subject: vec!["user".into()],
                traversal: false,
            },
            RelationDef {
                name: "viewer".into(),
                object: "resource".into(),
                subject: vec!["user".into(), "group".into()],
                traversal: false,
            },
            RelationDef {
                name: "member_of".into(),
                object: "group".into(),
                subject: vec!["user".into(), "group".into()],
                traversal: true,
            },
        ];
        let abac_attrs_user = vec![
            attr("department", AttrType::String),
            attr("clearance", AttrType::Int),
            attr("mfa", AttrType::Bool),
            attr("tags", AttrType::StringList),
        ];
        let abac_attrs_resource = vec![
            attr("classification", AttrType::String),
            attr("owner_id", AttrType::String),
            attr("tier", AttrType::Int),
        ];

        match self {
            Self::Rbac => ModelDefinition {
                entity_types: vec![user(vec![]), resource(vec![]), group.clone()],
                roles: rbac_roles,
                relations: vec![],
            },
            Self::Abac => ModelDefinition {
                entity_types: vec![user(abac_attrs_user), resource(abac_attrs_resource)],
                roles: vec![],
                relations: vec![],
            },
            Self::Rebac => ModelDefinition {
                entity_types: vec![user(vec![]), resource(vec![]), group.clone()],
                roles: vec![],
                relations: rebac_relations,
            },
            Self::Combined => ModelDefinition {
                entity_types: vec![user(abac_attrs_user), resource(abac_attrs_resource), group],
                roles: rbac_roles,
                relations: rebac_relations,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmEntity {
    pub entity_id: String,
    pub entity_type: String,
    pub attributes: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    pub subject: String,
    pub role: String,
    /// "" = namespace-wide.
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationTuple {
    pub object: String,
    pub relation: String,
    pub subject: String,
}

// ---------------------------------------------------------------------------
// Materializer: ADM records -> policy-engine DataLoader document
// ---------------------------------------------------------------------------

/// Compile the ADM into the exact `{"entities": [...]}` document the
/// policy-engine `DataLoader` consumes:
/// - attributes pass through (typed, already validated)
/// - UNSCOPED role bindings become `roles: [..]` + deduped
///   `permissions: [..]` attributes on the subject (RBAC-as-ABAC: one
///   interned lookup at eval). Scoped bindings are rejected at the API
///   until the materializer represents them (D2) — a scoped grant must
///   never silently widen to a global one.
/// - tuples become graph edges with DIRECTION decided by the relation
///   definition: traversal relations (member_of) land on the SUBJECT
///   (subject → object, the direction rebac::reachable walks); everything
///   else lands on the OBJECT (what rebac::related reads)
/// - entities referenced only by tuples get a minimal synthesized record
///   (type inferred from the relation definition)
///
/// Performance: single pass over each record set; all dedup via BTreeSet
/// (O(log n) inserts — a 100k-subject relation must not go quadratic).
/// Output ordering is deterministic (BTreeMap/BTreeSet) so checksums are
/// stable across publishes of identical data.
pub fn materialize(
    model: &ModelDefinition,
    entities: &[AdmEntity],
    bindings: &[RoleBinding],
    tuples: &[RelationTuple],
) -> serde_json::Value {
    use std::collections::BTreeSet;

    // subject -> (roles, permissions); BTreeSet = dedup + stable order.
    let mut roles_by_subject: BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> = BTreeMap::new();
    for b in bindings.iter().filter(|b| b.scope.is_empty()) {
        let entry = roles_by_subject.entry(b.subject.as_str()).or_default();
        entry.0.insert(b.role.as_str());
        if let Some(role) = model.role(&b.role) {
            entry.1.extend(role.permissions.iter().map(String::as_str));
        }
    }

    // carrier entity -> relation -> {targets}, direction per relation def.
    let mut rels_by_carrier: BTreeMap<&str, BTreeMap<&str, BTreeSet<&str>>> = BTreeMap::new();
    // carrier -> relation name, for type inference of synthesized entities.
    let mut carrier_relation: BTreeMap<&str, (&str, bool)> = BTreeMap::new();
    for t in tuples {
        let traversal = model.relation(&t.relation).is_some_and(|d| d.traversal);
        let (carrier, target) = if traversal {
            (t.subject.as_str(), t.object.as_str())
        } else {
            (t.object.as_str(), t.subject.as_str())
        };
        rels_by_carrier
            .entry(carrier)
            .or_default()
            .entry(t.relation.as_str())
            .or_default()
            .insert(target);
        carrier_relation
            .entry(carrier)
            .or_insert((t.relation.as_str(), traversal));
    }

    let known: HashMap<&str, ()> = entities
        .iter()
        .map(|e| (e.entity_id.as_str(), ()))
        .collect();

    let mut docs = Vec::with_capacity(entities.len() + rels_by_carrier.len());
    for e in entities {
        let mut attributes = e.attributes.clone();
        if let Some((roles, perms)) = roles_by_subject.get(e.entity_id.as_str()) {
            attributes.insert("roles".into(), serde_json::json!(roles));
            attributes.insert("permissions".into(), serde_json::json!(perms));
        }
        let mut doc = serde_json::json!({
            "id": e.entity_id,
            "type": e.entity_type,
            "attributes": attributes,
        });
        if let Some(rels) = rels_by_carrier.get(e.entity_id.as_str()) {
            doc["relationships"] = serde_json::json!(rels);
        }
        docs.push(doc);
    }

    // Synthesize records for tuple carriers with no explicit entity, so the
    // relationship graph is complete before anyone fills in attributes.
    for (carrier, rels) in &rels_by_carrier {
        if known.contains_key(carrier) {
            continue;
        }
        let inferred_type = carrier_relation
            .get(carrier)
            .and_then(|(rel, traversal)| {
                model.relation(rel).map(|d| {
                    if *traversal {
                        // Carrier is the tuple SUBJECT; pick its first
                        // declared subject type.
                        d.subject.first().cloned().unwrap_or_else(|| "user".into())
                    } else {
                        d.object.clone()
                    }
                })
            })
            .unwrap_or_else(|| "resource".to_string());
        docs.push(serde_json::json!({
            "id": carrier,
            "type": inferred_type,
            "attributes": {},
            "relationships": rels,
        }));
    }

    serde_json::json!({ "entities": docs })
}

/// Materialize ONE entity's current document for delta emission — the
/// changes API must not pay O(dataset) per poll. Inputs are the entity's
/// own record (None = referenced only by tuples), the bindings where it is
/// the subject, and every tuple touching it (either endpoint); direction
/// per relation definition, same rules as full materialize().
/// Returns None when nothing materializes (no record, no carried edges) —
/// the caller emits a tombstone delta.
pub fn materialize_one(
    model: &ModelDefinition,
    entity_id: &str,
    entity: Option<&AdmEntity>,
    bindings: &[RoleBinding],
    touching_tuples: &[RelationTuple],
) -> Option<serde_json::Value> {
    use std::collections::BTreeSet;

    // Edges THIS entity carries, direction-resolved.
    let mut rels: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for t in touching_tuples {
        let traversal = model.relation(&t.relation).is_some_and(|d| d.traversal);
        let (carrier, target) = if traversal {
            (t.subject.as_str(), t.object.as_str())
        } else {
            (t.object.as_str(), t.subject.as_str())
        };
        if carrier == entity_id {
            rels.entry(t.relation.as_str()).or_default().insert(target);
        }
    }

    if entity.is_none() && rels.is_empty() {
        return None;
    }

    let mut attributes = entity.map(|e| e.attributes.clone()).unwrap_or_default();
    let mut roles: BTreeSet<&str> = BTreeSet::new();
    let mut perms: BTreeSet<&str> = BTreeSet::new();
    for b in bindings.iter().filter(|b| b.scope.is_empty()) {
        roles.insert(b.role.as_str());
        if let Some(role) = model.role(&b.role) {
            perms.extend(role.permissions.iter().map(String::as_str));
        }
    }
    if !roles.is_empty() {
        attributes.insert("roles".into(), serde_json::json!(roles));
        attributes.insert("permissions".into(), serde_json::json!(perms));
    }

    let entity_type = entity.map(|e| e.entity_type.clone()).unwrap_or_else(|| {
        // Synthesized carrier: infer from any carried relation definition.
        rels.keys()
            .find_map(|r| {
                model.relation(r).map(|d| {
                    if d.traversal {
                        d.subject.first().cloned().unwrap_or_else(|| "user".into())
                    } else {
                        d.object.clone()
                    }
                })
            })
            .unwrap_or_else(|| "resource".into())
    });

    let mut doc = serde_json::json!({
        "id": entity_id,
        "type": entity_type,
        "attributes": attributes,
    });
    if !rels.is_empty() {
        doc["relationships"] = serde_json::json!(rels);
    }
    Some(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_template_has_all_three_vocabularies() {
        let m = DatastoreTemplate::Combined.seed_model();
        assert!(m.entity_type("user").is_some());
        assert!(m.role("admin").is_some());
        assert!(m.relation("member_of").is_some());
    }

    #[test]
    fn attribute_validation_is_type_strict() {
        let m = DatastoreTemplate::Combined.seed_model();
        let mut attrs = serde_json::Map::new();
        attrs.insert("clearance".into(), serde_json::json!("5")); // string, not int
        let err = m.validate_attributes("user", &attrs).unwrap_err();
        assert!(err.contains("must be Int"), "{err}");

        attrs.insert("clearance".into(), serde_json::json!(5));
        m.validate_attributes("user", &attrs).unwrap();

        attrs.insert("nonexistent".into(), serde_json::json!(true));
        assert!(m.validate_attributes("user", &attrs).is_err());
    }

    #[test]
    fn materialize_produces_loader_shape_with_roles_and_tuples() {
        let m = DatastoreTemplate::Combined.seed_model();
        let entities = vec![AdmEntity {
            entity_id: "alice".into(),
            entity_type: "user".into(),
            attributes: serde_json::Map::new(),
        }];
        let bindings = vec![RoleBinding {
            subject: "alice".into(),
            role: "editor".into(),
            scope: String::new(),
        }];
        let tuples = vec![RelationTuple {
            object: "doc-1".into(),
            relation: "owner".into(),
            subject: "alice".into(),
        }];

        let doc = materialize(&m, &entities, &bindings, &tuples);
        let ents = doc["entities"].as_array().unwrap();
        assert_eq!(ents.len(), 2, "alice + synthesized doc-1");

        let alice = ents.iter().find(|e| e["id"] == "alice").unwrap();
        assert_eq!(alice["attributes"]["roles"][0], "editor");
        assert_eq!(alice["attributes"]["permissions"][0], "resource:read");

        let doc1 = ents.iter().find(|e| e["id"] == "doc-1").unwrap();
        assert_eq!(doc1["type"], "resource", "type inferred from relation def");
        assert_eq!(doc1["relationships"]["owner"][0], "alice");
    }
}
