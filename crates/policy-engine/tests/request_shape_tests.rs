//! F1-s2: the agentic request-shape extensions on `PolicyRequest` —
//! optional `actor` and per-key `context_provenance` (taint). These are
//! additive and default-off; this suite pins that (a) pre-F1 payloads
//! deserialize unchanged, (b) the fail-untrusted taint rule holds, and
//! (c) `TrustLevel` orders correctly.

use policy_engine::{PolicyRequest, TrustLevel};
use std::collections::HashMap;

#[test]
fn legacy_json_without_new_fields_deserializes() {
    // A stored replay input / wire payload from before F1 — no `actor`, no
    // `context_provenance`. Must still parse (serde(default)).
    let json = r#"{"resource":"/doc","action":"read","context":{"principal":"alice"}}"#;
    let req: PolicyRequest = serde_json::from_str(json).expect("legacy payload parses");
    assert_eq!(req.resource, "/doc");
    assert_eq!(req.actor, None);
    assert!(req.context_provenance.is_none());
    // Taint mode OFF ⇒ every key reads as platform-trusted (pre-F1 behavior).
    assert_eq!(req.context_trust("principal"), TrustLevel::Platform);
    assert_eq!(req.context_trust("anything"), TrustLevel::Platform);
}

#[test]
fn taint_mode_defaults_unlabeled_keys_to_llm() {
    let mut provenance = HashMap::new();
    provenance.insert("approval_level".to_string(), TrustLevel::Platform);
    provenance.insert("user_note".to_string(), TrustLevel::Llm);

    let req = PolicyRequest {
        resource: "/doc".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),
        actor: Some("agent-1".to_string()),
        context_provenance: Some(provenance),
    };

    // Labeled keys read their label.
    assert_eq!(req.context_trust("approval_level"), TrustLevel::Platform);
    assert_eq!(req.context_trust("user_note"), TrustLevel::Llm);
    // Fail-untrusted: an UNLABELED key under taint mode is the floor, NOT
    // platform — a possibly-injected attribute must never be mistaken for a
    // derived one.
    assert_eq!(req.context_trust("smuggled"), TrustLevel::Llm);
}

#[test]
fn trust_levels_are_ordered_llm_lowest() {
    assert!(TrustLevel::Llm < TrustLevel::Verified);
    assert!(TrustLevel::Verified < TrustLevel::Platform);
    assert!(TrustLevel::Llm < TrustLevel::Platform);
    // A "require at least verified" gate rejects LLM, admits verified+platform.
    let min = TrustLevel::Verified;
    assert!(TrustLevel::Llm < min);
    assert!(TrustLevel::Verified >= min);
    assert!(TrustLevel::Platform >= min);
}

#[test]
fn new_fields_roundtrip_and_omit_when_absent() {
    // Absent optionals are skipped on the wire (no `actor:null` noise), so
    // the serialized default request is byte-identical to a pre-F1 one.
    let req = PolicyRequest {
        resource: "r".to_string(),
        action: "a".to_string(),
        context: HashMap::new(),
        ..Default::default()
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(
        !json.contains("actor"),
        "absent actor must be omitted: {json}"
    );
    assert!(
        !json.contains("context_provenance"),
        "absent provenance must be omitted: {json}"
    );

    // Present fields roundtrip.
    let mut prov = HashMap::new();
    prov.insert("k".to_string(), TrustLevel::Verified);
    let req2 = PolicyRequest {
        resource: "r".to_string(),
        action: "a".to_string(),
        context: HashMap::new(),
        actor: Some("agent".to_string()),
        context_provenance: Some(prov),
    };
    let round: PolicyRequest =
        serde_json::from_str(&serde_json::to_string(&req2).unwrap()).unwrap();
    assert_eq!(round.actor.as_deref(), Some("agent"));
    assert_eq!(round.context_trust("k"), TrustLevel::Verified);
}
