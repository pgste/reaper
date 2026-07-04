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
            },
            RelationDef {
                name: "viewer".into(),
                object: "resource".into(),
                subject: vec!["user".into(), "group".into()],
            },
            RelationDef {
                name: "member_of".into(),
                object: "group".into(),
                subject: vec!["user".into(), "group".into()],
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
/// - role bindings become `roles: [..]` + deduped `permissions: [..]`
///   attributes on the subject (RBAC-as-ABAC: one interned lookup at eval)
/// - tuples become `relationships: {relation: [subjects]}` on the OBJECT —
///   the shape `RelationshipGraph` ingests verbatim
/// - objects referenced only by tuples get a minimal synthesized entity
///   (type from the relation's `object` declaration)
///
/// Output is deterministic (BTreeMap ordering) so the checksum is stable.
pub fn materialize(
    model: &ModelDefinition,
    entities: &[AdmEntity],
    bindings: &[RoleBinding],
    tuples: &[RelationTuple],
) -> serde_json::Value {
    // subject -> (roles, permissions)
    let mut roles_by_subject: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
    for b in bindings {
        let entry = roles_by_subject.entry(b.subject.as_str()).or_default();
        if !entry.0.contains(&b.role.as_str()) {
            entry.0.push(b.role.as_str());
        }
        if let Some(role) = model.role(&b.role) {
            for p in &role.permissions {
                if !entry.1.contains(&p.as_str()) {
                    entry.1.push(p.as_str());
                }
            }
        }
    }

    // object -> relation -> [subjects]
    let mut rels_by_object: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    for t in tuples {
        let subjects = rels_by_object
            .entry(t.object.as_str())
            .or_default()
            .entry(t.relation.as_str())
            .or_default();
        if !subjects.contains(&t.subject.as_str()) {
            subjects.push(t.subject.as_str());
        }
    }

    let known: HashMap<&str, ()> = entities
        .iter()
        .map(|e| (e.entity_id.as_str(), ()))
        .collect();

    let mut docs = Vec::with_capacity(entities.len());
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
        if let Some(rels) = rels_by_object.get(e.entity_id.as_str()) {
            doc["relationships"] = serde_json::json!(rels);
        }
        docs.push(doc);
    }

    // Synthesize entities for tuple objects with no explicit record, so the
    // relationship graph is complete even before someone fills in attributes.
    for (object, rels) in &rels_by_object {
        if known.contains_key(object) {
            continue;
        }
        let inferred_type = rels
            .keys()
            .find_map(|r| model.relation(r).map(|d| d.object.clone()))
            .unwrap_or_else(|| "resource".to_string());
        docs.push(serde_json::json!({
            "id": object,
            "type": inferred_type,
            "attributes": {},
            "relationships": rels,
        }));
    }

    serde_json::json!({ "entities": docs })
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
