//! SIEM export formats for decision logs (round-2 E1, slice 1).
//!
//! [`DecisionLogEntry`] ships as NDJSON today (`to_ndjson`). SOC onboarding also
//! wants **shaped** records a SIEM ingests with zero custom parsers:
//!
//! - **OCSF** ([`DecisionLogEntry::to_ocsf`]) — the Open Cybersecurity Schema
//!   Framework. A policy allow/deny is an *authorization decision*, so it maps to
//!   the IAM-category **Authorize Session** class (`class_uid` 3003); allow/deny
//!   rides the universal `status_id` axis (Success/Failure). Every Reaper-specific
//!   field that has no OCSF home is preserved under the schema's blessed
//!   `unmapped` object, so nothing is lost and the record still validates.
//!   Ingested natively by Amazon Security Lake, Splunk, Snowflake, etc.
//! - **CEF** ([`DecisionLogEntry::to_cef`]) — ArcSight Common Event Format, the
//!   lingua franca for legacy SIEM collectors, with correct header/extension
//!   escaping.
//!
//! Shaping is transport-agnostic and lives here in `policy-engine` so both the
//! agent path and the control-plane push path (slice 3) emit identical records.
//! Redaction already happened at capture (`decision_privacy`), so these mappers
//! inherit it for free — and they deliberately omit the potentially-large,
//! possibly-encrypted `input_data`/`replay_input` blobs, which stay in the NDJSON
//! stream for the full-fidelity/replay path.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::decision_log::DecisionLogEntry;

/// OCSF schema version these records target. Bump deliberately — golden fixtures
/// pin it, so a change is a reviewed diff.
pub const OCSF_VERSION: &str = "1.1.0";

/// OCSF class: **Authorize Session** (Identity & Access Management category).
const OCSF_CATEGORY_UID: u32 = 3;
const OCSF_CATEGORY_NAME: &str = "Identity & Access Management";
const OCSF_CLASS_UID: u32 = 3003;
const OCSF_CLASS_NAME: &str = "Authorize Session";
/// The decision isn't one of the class's Assign-Privileges/Groups activities, so
/// it is honestly reported as `Other` with a descriptive `activity_name`.
const OCSF_ACTIVITY_ID: u32 = 99;
const OCSF_ACTIVITY_NAME: &str = "Authorization Decision";

/// A SIEM export format for a decision record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// The native Reaper NDJSON record (`to_ndjson`), unshaped.
    Ndjson,
    /// OCSF Authorize Session JSON (`to_ocsf`).
    Ocsf,
    /// ArcSight CEF (`to_cef`).
    Cef,
}

impl ExportFormat {
    /// Parse a case-insensitive format name; `None` if unrecognized.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "ndjson" | "json" => Some(Self::Ndjson),
            "ocsf" => Some(Self::Ocsf),
            "cef" => Some(Self::Cef),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ndjson => "ndjson",
            Self::Ocsf => "ocsf",
            Self::Cef => "cef",
        }
    }
}

impl DecisionLogEntry {
    /// Epoch milliseconds from the RFC3339 `timestamp`, or 0 if it can't be
    /// parsed (the audit/export path must never panic).
    fn time_millis(&self) -> i64 {
        chrono::DateTime::parse_from_rfc3339(&self.timestamp)
            .map(|d| d.timestamp_millis())
            .unwrap_or(0)
    }

    /// `(status_id, status, severity_id, severity)` for the decision. Allow/deny
    /// is the OCSF `status_id` axis; a deny is scored one notch above
    /// informational so a SIEM can alert on it without parsing Reaper fields.
    fn ocsf_status_severity(&self) -> (u32, &'static str, u32, &'static str) {
        match self.decision.as_str() {
            "allow" => (1, "Success", 1, "Informational"),
            "deny" => (2, "Failure", 2, "Low"),
            // "log" and anything else: an outcome we neither grant nor refuse.
            _ => (99, "Other", 1, "Informational"),
        }
    }

    /// Map this decision to an OCSF **Authorize Session** (`class_uid` 3003)
    /// event. Reaper-specific fields live under `unmapped`. See the module docs.
    pub fn to_ocsf(&self) -> Value {
        let (status_id, status, severity_id, severity) = self.ocsf_status_severity();

        // Reaper fields with no first-class OCSF home — preserved, never dropped.
        let mut unmapped = Map::new();
        unmapped.insert("action".to_string(), json!(self.action));
        unmapped.insert("decision".to_string(), json!(self.decision));
        unmapped.insert("policy_id".to_string(), json!(self.policy_id));
        unmapped.insert("policy_name".to_string(), json!(self.policy_name));
        unmapped.insert(
            "evaluation_time_ns".to_string(),
            json!(self.evaluation_time_ns),
        );
        unmapped.insert("cache_hit".to_string(), json!(self.cache_hit));
        if let Some(v) = &self.policy_version {
            unmapped.insert("policy_version".to_string(), json!(v));
        }
        if let Some(r) = &self.matched_rule {
            unmapped.insert("matched_rule".to_string(), json!(r));
        }
        if let Some(a) = &self.agent_id {
            unmapped.insert("agent_id".to_string(), json!(a));
        }
        if let Some(v) = self.data_version {
            unmapped.insert("data_version".to_string(), json!(v));
        }
        if let Some(v) = self.model_version {
            unmapped.insert("model_version".to_string(), json!(v));
        }
        if let Some(c) = &self.data_checksum {
            unmapped.insert("data_checksum".to_string(), json!(c));
        }
        if self.data_stale {
            unmapped.insert("data_stale".to_string(), json!(true));
        }
        if self.seq != 0 {
            unmapped.insert("seq".to_string(), json!(self.seq));
        }
        if !self.chain_id.is_empty() {
            unmapped.insert("chain_id".to_string(), json!(self.chain_id));
        }
        // Preserve request context (already redacted at capture) for the analyst.
        if !self.context.is_empty() {
            unmapped.insert("context".to_string(), json!(self.context));
        }

        let mut metadata = json!({
            "version": OCSF_VERSION,
            "product": { "name": "Reaper", "vendor_name": "Reaper" },
            "uid": self.decision_id,
            "log_name": "decision_log",
        });
        if let Some(trace) = &self.trace_id {
            metadata["correlation_uid"] = json!(trace);
        }

        json!({
            "activity_id": OCSF_ACTIVITY_ID,
            "activity_name": OCSF_ACTIVITY_NAME,
            "category_uid": OCSF_CATEGORY_UID,
            "category_name": OCSF_CATEGORY_NAME,
            "class_uid": OCSF_CLASS_UID,
            "class_name": OCSF_CLASS_NAME,
            "type_uid": OCSF_CLASS_UID * 100 + OCSF_ACTIVITY_ID,
            "time": self.time_millis(),
            "severity_id": severity_id,
            "severity": severity,
            "status_id": status_id,
            "status": status,
            "status_detail": self.decision,
            "message": format!(
                "Authorization {}: {} on {}",
                self.decision, self.action, self.resource
            ),
            "metadata": metadata,
            "actor": {
                "user": { "name": self.principal, "uid": self.principal, "type": "User" }
            },
            // The action being authorized reads naturally as a requested privilege.
            "privileges": [self.action],
            "resources": [ { "name": self.resource, "type": "resource" } ],
            "unmapped": Value::Object(unmapped),
        })
    }

    /// OCSF record as a single compact NDJSON line (streaming/HEC-friendly).
    pub fn to_ocsf_ndjson(&self) -> Result<String, String> {
        serde_json::to_string(&self.to_ocsf()).map_err(|e| e.to_string())
    }

    /// Map this decision to an ArcSight **CEF** line. Header fields escape `\`
    /// and `|`; extension values escape `\`, `=`, and newlines — per the CEF
    /// spec, so a value containing those characters can't break the record.
    pub fn to_cef(&self) -> String {
        let severity = match self.decision.as_str() {
            "allow" => 3,
            "deny" => 7,
            _ => 3,
        };
        let signature = format!("authz-{}", self.decision);
        let name = format!(
            "Authorization {}: {} on {}",
            self.decision, self.action, self.resource
        );

        // CEF extension: standard keys where they exist, custom cs*/cn* otherwise.
        let mut ext: Vec<(String, String)> = vec![
            ("rt".to_string(), self.time_millis().to_string()),
            ("suser".to_string(), self.principal.clone()),
            ("act".to_string(), self.action.clone()),
            ("outcome".to_string(), self.decision.clone()),
            ("request".to_string(), self.resource.clone()),
            ("externalId".to_string(), self.decision_id.clone()),
            ("cs1Label".to_string(), "PolicyName".to_string()),
            ("cs1".to_string(), self.policy_name.clone()),
            ("cs2Label".to_string(), "PolicyId".to_string()),
            ("cs2".to_string(), self.policy_id.clone()),
            ("cn1Label".to_string(), "EvaluationTimeNs".to_string()),
            ("cn1".to_string(), self.evaluation_time_ns.to_string()),
        ];
        if let Some(rule) = &self.matched_rule {
            ext.push(("cs3Label".to_string(), "MatchedRule".to_string()));
            ext.push(("cs3".to_string(), rule.clone()));
        }
        if let Some(agent) = &self.agent_id {
            ext.push(("cs4Label".to_string(), "AgentId".to_string()));
            ext.push(("cs4".to_string(), agent.clone()));
        }
        if let Some(trace) = &self.trace_id {
            ext.push(("cs5Label".to_string(), "TraceId".to_string()));
            ext.push(("cs5".to_string(), trace.clone()));
        }
        if let Some(v) = self.data_version {
            ext.push(("cn2Label".to_string(), "DataVersion".to_string()));
            ext.push(("cn2".to_string(), v.to_string()));
        }

        let ext_str = ext
            .iter()
            .map(|(k, v)| format!("{k}={}", cef_escape_extension(v)))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "CEF:0|Reaper|Reaper|{}|{}|{}|{}|{}",
            cef_escape_header(env!("CARGO_PKG_VERSION")),
            cef_escape_header(&signature),
            cef_escape_header(&name),
            severity,
            ext_str,
        )
    }

    /// Serialize this record in `format` as one line ready for a SIEM sink.
    pub fn export(&self, format: ExportFormat) -> Result<String, String> {
        match format {
            ExportFormat::Ndjson => self.to_ndjson(),
            ExportFormat::Ocsf => self.to_ocsf_ndjson(),
            ExportFormat::Cef => Ok(self.to_cef()),
        }
    }
}

/// Escape a CEF header field: backslash and pipe are the only metacharacters.
fn cef_escape_header(s: &str) -> String {
    s.replace('\\', "\\\\").replace('|', "\\|")
}

/// Escape a CEF extension value: backslash, `=`, and newlines (a newline would
/// otherwise terminate the record).
fn cef_escape_extension(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('=', "\\=")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A fully-populated, deterministic entry (no random ids/timestamps) so the
    /// mapped output is byte-stable and can be pinned as a golden fixture.
    fn golden_entry() -> DecisionLogEntry {
        let mut context = HashMap::new();
        context.insert("region".to_string(), json!("eu-west-1"));
        let mut e = DecisionLogEntry::new(
            "alice@example.com".to_string(),
            "read".to_string(),
            "/records/42".to_string(),
            "deny".to_string(),
            "pol-uuid-1".to_string(),
            "records-policy".to_string(),
        );
        e.timestamp = "2026-07-15T10:30:00Z".to_string();
        e.decision_id = "dec-uuid-1".to_string();
        e.trace_id = Some("trace-abc".to_string());
        e.context = context;
        e.policy_version = Some("3".to_string());
        e.evaluation_time_ns = 450;
        e.agent_id = Some("agent-1".to_string());
        e.matched_rule = Some("deny-pii".to_string());
        e.data_version = Some(7);
        e.model_version = Some(2);
        e
    }

    #[test]
    fn ocsf_maps_authorize_session_class_and_status() {
        let o = golden_entry().to_ocsf();
        assert_eq!(o["class_uid"], 3003);
        assert_eq!(o["category_uid"], 3);
        assert_eq!(o["type_uid"], 300399);
        // deny → Failure on the universal status axis.
        assert_eq!(o["status_id"], 2);
        assert_eq!(o["status"], "Failure");
        assert_eq!(o["severity_id"], 2);
        assert_eq!(o["metadata"]["version"], OCSF_VERSION);
        assert_eq!(o["metadata"]["uid"], "dec-uuid-1");
        assert_eq!(o["metadata"]["correlation_uid"], "trace-abc");
        assert_eq!(o["actor"]["user"]["name"], "alice@example.com");
        assert_eq!(o["privileges"][0], "read");
        assert_eq!(o["resources"][0]["name"], "/records/42");
        // Reaper-specific fields preserved under `unmapped`, not dropped.
        assert_eq!(o["unmapped"]["policy_name"], "records-policy");
        assert_eq!(o["unmapped"]["matched_rule"], "deny-pii");
        assert_eq!(o["unmapped"]["data_version"], 7);
        assert_eq!(o["unmapped"]["evaluation_time_ns"], 450);
        assert_eq!(o["unmapped"]["context"]["region"], "eu-west-1");
    }

    #[test]
    fn ocsf_allow_is_success_and_informational() {
        let mut e = golden_entry();
        e.decision = "allow".to_string();
        let o = e.to_ocsf();
        assert_eq!(o["status_id"], 1);
        assert_eq!(o["status"], "Success");
        assert_eq!(o["severity_id"], 1);
    }

    #[test]
    fn ocsf_line_is_single_line_json() {
        let line = golden_entry().to_ocsf_ndjson().unwrap();
        assert!(
            !line.contains('\n'),
            "NDJSON line must not contain newlines"
        );
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["class_uid"], 3003);
    }

    #[test]
    fn cef_header_and_extension_are_shaped() {
        let cef = golden_entry().to_cef();
        assert!(cef.starts_with("CEF:0|Reaper|Reaper|"), "{cef}");
        // At least the 7 header pipes precede the extension.
        assert!(cef.matches('|').count() >= 7, "{cef}");
        assert!(cef.contains("|authz-deny|"), "{cef}");
        assert!(cef.contains("suser=alice@example.com"), "{cef}");
        assert!(cef.contains("act=read"), "{cef}");
        assert!(cef.contains("outcome=deny"), "{cef}");
        assert!(cef.contains("request=/records/42"), "{cef}");
        assert!(
            cef.contains("cs1Label=PolicyName cs1=records-policy"),
            "{cef}"
        );
        assert!(cef.contains("cn1Label=EvaluationTimeNs cn1=450"), "{cef}");
        assert!(cef.contains("cs3Label=MatchedRule cs3=deny-pii"), "{cef}");
    }

    #[test]
    fn cef_escapes_metacharacters() {
        let mut e = golden_entry();
        // Values carrying CEF metacharacters must not break the record.
        e.resource = "a=b|c\\d".to_string();
        e.action = "wr=ite".to_string();
        let cef = e.to_cef();
        // Extension '=' inside a value is escaped.
        assert!(cef.contains("act=wr\\=ite"), "{cef}");
        assert!(cef.contains("request=a\\=b|c\\\\d"), "{cef}");
        // The header (name) escapes the pipe from the resource.
        assert!(cef.contains("c\\\\d"), "{cef}");
    }

    #[test]
    fn export_dispatches_by_format() {
        let e = golden_entry();
        assert!(e
            .export(ExportFormat::Ndjson)
            .unwrap()
            .contains("\"principal\""));
        assert!(e
            .export(ExportFormat::Ocsf)
            .unwrap()
            .contains("\"class_uid\":3003"));
        assert!(e.export(ExportFormat::Cef).unwrap().starts_with("CEF:0|"));
    }

    #[test]
    fn ocsf_matches_golden_fixture() {
        // The reviewable sign-off artifact: the full OCSF shape for a
        // representative deny. `time` is epoch-derived, so verify it
        // independently (via a fresh chrono parse, not the code under test) and
        // normalize before the structural compare — key order is irrelevant
        // since both sides are compared as parsed `Value`s.
        let mut actual = golden_entry().to_ocsf();
        let want_time = chrono::DateTime::parse_from_rfc3339("2026-07-15T10:30:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(actual["time"], json!(want_time));
        actual["time"] = json!(0);

        let expected: Value =
            serde_json::from_str(include_str!("testdata/decision_ocsf.json")).unwrap();
        assert_eq!(
            actual, expected,
            "OCSF output drifted from the golden fixture (src/testdata/decision_ocsf.json)"
        );
    }

    #[test]
    fn export_format_parse_roundtrips() {
        for f in [ExportFormat::Ndjson, ExportFormat::Ocsf, ExportFormat::Cef] {
            assert_eq!(ExportFormat::parse(f.as_str()), Some(f));
        }
        assert_eq!(ExportFormat::parse("OCSF"), Some(ExportFormat::Ocsf));
        assert_eq!(ExportFormat::parse("json"), Some(ExportFormat::Ndjson));
        assert_eq!(ExportFormat::parse("nope"), None);
    }
}
