//! Adversarial tests for the capability core (F1-s1). Every rejection path
//! is exercised: expiry, pre-validity, tampering of each claim, wrong key,
//! algorithm confusion, widened grants/windows on attenuation, revocation
//! of leaf AND ancestor, malformed signatures, and pattern-subset edges.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use reaper_core::bundle_signing::{SigAlgorithm, SigningKey, VerifyingKey};
use reaper_core::capability::{
    attenuate, issue, pattern_covers, pattern_matches, Capability, CapabilityError, Grant,
};
use std::collections::HashSet;

const NOW: i64 = 1_800_000_000;

fn keypair() -> (SigningKey, VerifyingKey) {
    let sk = SigningKey::generate(SigAlgorithm::Ed25519Sha256);
    let vk = VerifyingKey::from_hex(SigAlgorithm::Ed25519Sha256, &sk.public_key_hex()).unwrap();
    (sk, vk)
}

fn root_cap(sk: &SigningKey) -> Capability {
    issue(
        sk,
        "key-1",
        "alice",
        "agent-orchestrator",
        vec![Grant::new("read", "doc/*"), Grant::new("*", "tmp/scratch")],
        NOW - 60,
        NOW + 300,
    )
    .unwrap()
}

fn none() -> HashSet<String> {
    HashSet::new()
}

#[test]
fn round_trip_verifies_and_authorizes_subset_only() {
    let (sk, vk) = keypair();
    let cap = root_cap(&sk);
    cap.verify_at(&vk, "key-1", NOW, &none()).unwrap();

    assert!(cap.authorizes("read", "doc/plan.md"));
    assert!(cap.authorizes("delete", "tmp/scratch"));
    assert!(
        !cap.authorizes("write", "doc/plan.md"),
        "action outside grants"
    );
    assert!(
        !cap.authorizes("read", "secrets/key"),
        "resource outside grants"
    );
}

#[test]
fn expired_and_not_yet_valid_fail_closed() {
    let (sk, vk) = keypair();
    let cap = root_cap(&sk);
    assert!(matches!(
        cap.verify_at(&vk, "key-1", NOW + 301, &none()),
        Err(CapabilityError::Expired { .. })
    ));
    assert!(matches!(
        cap.verify_at(&vk, "key-1", NOW - 61, &none()),
        Err(CapabilityError::NotYetValid { .. })
    ));
}

#[test]
fn every_tampered_claim_breaks_the_signature() {
    let (sk, vk) = keypair();
    let base = root_cap(&sk);

    let mut tampered: Vec<Capability> = Vec::new();
    let mut c = base.clone();
    c.subject = "mallory".into();
    tampered.push(c);
    let mut c = base.clone();
    c.actor = "agent-evil".into();
    tampered.push(c);
    let mut c = base.clone();
    c.grants.push(Grant::new("*", "*"));
    tampered.push(c);
    let mut c = base.clone();
    c.expires_at += 3600;
    tampered.push(c);
    let mut c = base.clone();
    c.not_before -= 3600;
    tampered.push(c);
    let mut c = base.clone();
    c.ancestry = vec![];
    c.id = "someone-elses-id".into();
    tampered.push(c);

    for cap in tampered {
        assert!(
            matches!(
                cap.verify_at(&vk, "key-1", NOW, &none()),
                Err(CapabilityError::BadSignature)
            ),
            "tampered capability must fail signature: {cap:?}"
        );
    }
}

#[test]
fn wrong_key_and_key_id_pin_fail() {
    let (sk, _) = keypair();
    let (_, other_vk) = keypair();
    let cap = root_cap(&sk);
    assert!(matches!(
        cap.verify_at(&other_vk, "key-1", NOW, &none()),
        Err(CapabilityError::BadSignature)
    ));
    let (_, vk) = (
        0,
        VerifyingKey::from_hex(SigAlgorithm::Ed25519Sha256, &sk.public_key_hex()).unwrap(),
    );
    assert!(matches!(
        cap.verify_at(&vk, "key-2", NOW, &none()),
        Err(CapabilityError::KeyMismatch { .. })
    ));
}

#[test]
fn algorithm_confusion_is_rejected_cleanly() {
    let (sk, _) = keypair();
    let cap = root_cap(&sk);
    let p256 = SigningKey::generate(SigAlgorithm::EcdsaP256Sha256);
    let p256_vk =
        VerifyingKey::from_hex(SigAlgorithm::EcdsaP256Sha256, &p256.public_key_hex()).unwrap();
    assert!(matches!(
        cap.verify_at(&p256_vk, "key-1", NOW, &none()),
        Err(CapabilityError::UnknownAlgorithm(_))
    ));
}

#[test]
fn malformed_signature_is_malformed_not_panic() {
    let (sk, vk) = keypair();
    let mut cap = root_cap(&sk);
    cap.signature = "not-hex!!".into();
    assert!(matches!(
        cap.verify_at(&vk, "key-1", NOW, &none()),
        Err(CapabilityError::Malformed(_))
    ));
}

#[test]
fn attenuation_narrows_and_verifies() {
    let (sk, vk) = keypair();
    let parent = root_cap(&sk);
    let child = attenuate(
        &parent,
        &sk,
        "key-1",
        "agent-subtask",
        vec![Grant::new("read", "doc/reports/*")],
        NOW,
        NOW + 60,
    )
    .unwrap();

    child.verify_at(&vk, "key-1", NOW + 30, &none()).unwrap();
    assert_eq!(child.subject, "alice", "subject lineage is inherited");
    assert_eq!(child.ancestry, vec![parent.id.clone()]);
    assert!(child.authorizes("read", "doc/reports/q3.md"));
    assert!(!child.authorizes("read", "doc/plan.md"), "narrowed away");
}

#[test]
fn attenuation_cannot_widen_grants_or_window() {
    let (sk, _) = keypair();
    let parent = root_cap(&sk);

    // Widened action on a covered resource.
    assert!(matches!(
        attenuate(
            &parent,
            &sk,
            "key-1",
            "a",
            vec![Grant::new("write", "doc/x")],
            NOW,
            NOW + 60
        ),
        Err(CapabilityError::WidenedGrant { .. })
    ));
    // Wildcard child under literal-prefix parent that would escape it.
    assert!(matches!(
        attenuate(
            &parent,
            &sk,
            "key-1",
            "a",
            vec![Grant::new("read", "*")],
            NOW,
            NOW + 60
        ),
        Err(CapabilityError::WidenedGrant { .. })
    ));
    // Literal parent cannot cover wildcard child on actions.
    assert!(matches!(
        attenuate(
            &parent,
            &sk,
            "key-1",
            "a",
            vec![Grant::new("rea*", "doc/a")],
            NOW,
            NOW + 60
        ),
        Err(CapabilityError::WidenedGrant { .. })
    ));
    // Window extension.
    assert!(matches!(
        attenuate(
            &parent,
            &sk,
            "key-1",
            "a",
            vec![Grant::new("read", "doc/a")],
            NOW,
            NOW + 3600
        ),
        Err(CapabilityError::WidenedWindow)
    ));
    assert!(matches!(
        attenuate(
            &parent,
            &sk,
            "key-1",
            "a",
            vec![Grant::new("read", "doc/a")],
            NOW - 3600,
            NOW
        ),
        Err(CapabilityError::WidenedWindow)
    ));
}

#[test]
fn revoking_an_ancestor_kills_the_whole_chain() {
    let (sk, vk) = keypair();
    let root = root_cap(&sk);
    let mid = attenuate(
        &root,
        &sk,
        "key-1",
        "agent-a",
        vec![Grant::new("read", "doc/*")],
        NOW,
        NOW + 120,
    )
    .unwrap();
    let leaf = attenuate(
        &mid,
        &sk,
        "key-1",
        "agent-b",
        vec![Grant::new("read", "doc/x")],
        NOW,
        NOW + 60,
    )
    .unwrap();

    leaf.verify_at(&vk, "key-1", NOW, &none()).unwrap();

    // Revoke the leaf: only the leaf dies.
    let mut revoked = HashSet::new();
    revoked.insert(leaf.id.clone());
    assert!(matches!(
        leaf.verify_at(&vk, "key-1", NOW, &revoked),
        Err(CapabilityError::Revoked { .. })
    ));
    mid.verify_at(&vk, "key-1", NOW, &revoked).unwrap();

    // Revoke the ROOT: every descendant dies with it.
    let mut revoked = HashSet::new();
    revoked.insert(root.id.clone());
    assert!(matches!(
        leaf.verify_at(&vk, "key-1", NOW, &revoked),
        Err(CapabilityError::Revoked { .. })
    ));
    assert!(matches!(
        mid.verify_at(&vk, "key-1", NOW, &revoked),
        Err(CapabilityError::Revoked { .. })
    ));
}

#[test]
fn empty_grants_verify_but_authorize_nothing() {
    let (sk, vk) = keypair();
    let cap = issue(&sk, "key-1", "alice", "agent", vec![], NOW, NOW + 60).unwrap();
    cap.verify_at(&vk, "key-1", NOW, &none()).unwrap();
    assert!(!cap.authorizes("read", "anything"));
}

#[test]
fn invalid_window_rejected_at_issue_and_verify() {
    let (sk, _) = keypair();
    assert!(matches!(
        issue(&sk, "key-1", "s", "a", vec![], NOW + 10, NOW),
        Err(CapabilityError::InvalidWindow)
    ));
}

#[test]
fn pattern_semantics_edges() {
    // matches
    assert!(pattern_matches("*", "anything"));
    assert!(pattern_matches("doc/*", "doc/a/b"));
    assert!(pattern_matches("doc/*", "doc/"));
    assert!(!pattern_matches("doc/*", "docs"));
    assert!(pattern_matches("read", "read"));
    assert!(
        !pattern_matches("read", "reader"),
        "literal is not a prefix"
    );
    // covers
    assert!(pattern_covers("*", "doc/*"));
    assert!(pattern_covers("doc/*", "doc/a*"));
    assert!(pattern_covers("doc/*", "doc/a"));
    assert!(
        !pattern_covers("doc/a*", "doc/*"),
        "child prefix escapes parent"
    );
    assert!(
        !pattern_covers("doc/a", "doc/a*"),
        "literal cannot cover wildcard"
    );
    assert!(pattern_covers("doc/a", "doc/a"));
    assert!(!pattern_covers("doc/*", "dob/*"));
}

#[test]
fn p256_round_trip_works_too() {
    let sk = SigningKey::generate(SigAlgorithm::EcdsaP256Sha256);
    let vk = VerifyingKey::from_hex(SigAlgorithm::EcdsaP256Sha256, &sk.public_key_hex()).unwrap();
    let cap = issue(
        &sk,
        "kp",
        "alice",
        "agent",
        vec![Grant::new("read", "r")],
        NOW,
        NOW + 5,
    )
    .unwrap();
    cap.verify_at(&vk, "kp", NOW, &none()).unwrap();
    assert!(cap.authorizes("read", "r"));
}
