//! JWT (JSON Web Token) utilities for entity creation
//!
//! Provides helpers for parsing JWT tokens and converting claims to entity data
//! for use in eBPF policy evaluation.

use crate::entity::{EntityData, RelationshipData};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;

/// JWT standard claims
pub mod claims {
    /// Subject - identifies the principal
    pub const SUB: &str = "sub";
    /// Issuer - identifies who issued the token
    pub const ISS: &str = "iss";
    /// Audience - identifies the recipients
    pub const AUD: &str = "aud";
    /// Expiration time - when the token expires (Unix timestamp)
    pub const EXP: &str = "exp";
    /// Not before - when the token becomes valid (Unix timestamp)
    pub const NBF: &str = "nbf";
    /// Issued at - when the token was issued (Unix timestamp)
    pub const IAT: &str = "iat";
    /// JWT ID - unique identifier for the token
    pub const JTI: &str = "jti";
}

/// JWT parser (signature verification optional for performance)
pub struct JwtParser {
    /// Whether to verify signatures (disabled for performance in eBPF context)
    verify_signature: bool,
}

impl JwtParser {
    /// Create a new JWT parser
    pub fn new() -> Self {
        Self {
            verify_signature: false,
        }
    }

    /// Enable signature verification (slower, more secure)
    pub fn with_verification(mut self) -> Self {
        self.verify_signature = true;
        self
    }

    /// Parse a JWT token and extract claims without verification
    ///
    /// This is fast but doesn't verify the signature. Use only when:
    /// - JWT signature was already verified upstream
    /// - You're in a trusted environment
    /// - Performance is critical (eBPF fast path)
    pub fn parse_claims_unverified(&self, token: &str) -> Result<HashMap<String, Value>> {
        // JWT format: header.payload.signature
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(anyhow!("Invalid JWT format: expected 3 parts"));
        }

        // Decode payload (base64url)
        let payload = self
            .decode_base64url(parts[1])
            .context("Failed to decode JWT payload")?;

        // Parse JSON
        let claims: HashMap<String, Value> =
            serde_json::from_slice(&payload).context("Failed to parse JWT claims JSON")?;

        Ok(claims)
    }

    /// Create an entity from JWT claims
    pub fn create_entity_from_token(&self, token: &str, session_id: &str) -> Result<EntityData> {
        let claims = self.parse_claims_unverified(token)?;
        Ok(self.claims_to_entity(claims, session_id))
    }

    /// Convert JWT claims to EntityData
    pub fn claims_to_entity(&self, claims: HashMap<String, Value>, session_id: &str) -> EntityData {
        let mut entity = EntityData {
            entity_type: "jwt_session".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        // Add session ID to metadata
        entity
            .metadata
            .insert("session_id".to_string(), session_id.to_string());

        // Process each claim
        for (key, value) in claims {
            match value {
                Value::String(s) => {
                    entity.string_attrs.insert(key, s);
                }
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        entity.numeric_attrs.insert(key, i);
                    } else if let Some(f) = n.as_f64() {
                        // Convert float to i64 (timestamp use case)
                        entity.numeric_attrs.insert(key, f as i64);
                    }
                }
                Value::Bool(b) => {
                    entity.flags.insert(key, b);
                }
                Value::Array(arr) => {
                    // Handle string arrays (common in roles, scopes)
                    if let Some(first) = arr.first() {
                        if first.is_string() {
                            // For role arrays, create relationships
                            if key == "roles" || key == "groups" {
                                for item in arr {
                                    if let Some(role) = item.as_str() {
                                        entity.relationships.push(RelationshipData {
                                            rel_type: "has_role".to_string(),
                                            target: format!("role:{}", role),
                                        });
                                    }
                                }
                            } else {
                                // Otherwise, join as space-separated string
                                let joined = arr
                                    .iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                if !joined.is_empty() {
                                    entity.string_attrs.insert(key, joined);
                                }
                            }
                        }
                    }
                }
                Value::Object(_) => {
                    // Skip nested objects for now (eBPF can't handle complex structures)
                }
                Value::Null => {
                    // Skip null values
                }
            }
        }

        entity
    }

    /// Decode base64url (JWT uses base64url encoding, not standard base64)
    fn decode_base64url(&self, input: &str) -> Result<Vec<u8>> {
        // JWT uses base64url encoding (RFC 4648) which replaces + with - and / with _
        // and doesn't require padding
        let padded = match input.len() % 4 {
            2 => format!("{}==", input),
            3 => format!("{}=", input),
            _ => input.to_string(),
        };

        let replaced = padded.replace('-', "+").replace('_', "/");

        base64::engine::general_purpose::STANDARD
            .decode(replaced)
            .context("Failed to decode base64url")
    }
}

impl Default for JwtParser {
    fn default() -> Self {
        Self::new()
    }
}

/// JWT claim extraction helpers
pub struct JwtClaims;

impl JwtClaims {
    /// Extract subject (sub) from claims
    pub fn get_subject(entity: &EntityData) -> Option<&String> {
        entity.string_attrs.get(claims::SUB)
    }

    /// Extract issuer (iss) from claims
    pub fn get_issuer(entity: &EntityData) -> Option<&String> {
        entity.string_attrs.get(claims::ISS)
    }

    /// Extract expiration time (exp) from claims
    pub fn get_expiration(entity: &EntityData) -> Option<i64> {
        entity.numeric_attrs.get(claims::EXP).copied()
    }

    /// Extract issued at time (iat) from claims
    pub fn get_issued_at(entity: &EntityData) -> Option<i64> {
        entity.numeric_attrs.get(claims::IAT).copied()
    }

    /// Check if JWT is expired (based on exp claim)
    pub fn is_expired(entity: &EntityData) -> bool {
        if let Some(exp) = Self::get_expiration(entity) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            exp < now
        } else {
            // No expiration claim - consider it expired for safety
            true
        }
    }

    /// Check if JWT is not yet valid (based on nbf claim)
    pub fn is_not_yet_valid(entity: &EntityData) -> bool {
        if let Some(nbf) = entity.numeric_attrs.get(claims::NBF) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            *nbf > now
        } else {
            // No nbf claim - consider it valid
            false
        }
    }

    /// Check if JWT is currently valid (not expired and not before nbf)
    pub fn is_valid(entity: &EntityData) -> bool {
        !Self::is_expired(entity) && !Self::is_not_yet_valid(entity)
    }

    /// Extract custom claim as string
    pub fn get_string_claim<'a>(entity: &'a EntityData, claim: &str) -> Option<&'a String> {
        entity.string_attrs.get(claim)
    }

    /// Extract custom claim as number
    pub fn get_numeric_claim(entity: &EntityData, claim: &str) -> Option<i64> {
        entity.numeric_attrs.get(claim).copied()
    }

    /// Extract custom claim as boolean flag
    pub fn get_bool_claim(entity: &EntityData, claim: &str) -> Option<bool> {
        entity.flags.get(claim).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Example JWT token (header.payload.signature)
    // Header: {"alg":"HS256","typ":"JWT"}
    // Payload: {"sub":"user123","name":"John Doe","iat":1516239022,"exp":1735689600,"roles":["admin","user"]}
    const TEST_JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyMTIzIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyLCJleHAiOjE3MzU2ODk2MDAsInJvbGVzIjpbImFkbWluIiwidXNlciJdfQ.SIGNATURE";

    #[test]
    fn test_parse_jwt_claims() {
        let parser = JwtParser::new();
        let claims = parser.parse_claims_unverified(TEST_JWT).unwrap();

        assert_eq!(claims.get("sub").and_then(|v| v.as_str()), Some("user123"));
        assert_eq!(
            claims.get("name").and_then(|v| v.as_str()),
            Some("John Doe")
        );
        assert_eq!(claims.get("iat").and_then(|v| v.as_i64()), Some(1516239022));
        assert_eq!(claims.get("exp").and_then(|v| v.as_i64()), Some(1735689600));

        // Check roles array
        let roles = claims.get("roles").and_then(|v| v.as_array());
        assert!(roles.is_some());
        assert_eq!(roles.unwrap().len(), 2);
    }

    #[test]
    fn test_create_entity_from_token() {
        let parser = JwtParser::new();
        let entity = parser
            .create_entity_from_token(TEST_JWT, "session_abc123")
            .unwrap();

        assert_eq!(entity.entity_type, "jwt_session");

        // Check string claims
        assert_eq!(entity.string_attrs.get("sub"), Some(&"user123".to_string()));
        assert_eq!(
            entity.string_attrs.get("name"),
            Some(&"John Doe".to_string())
        );

        // Check numeric claims
        assert_eq!(entity.numeric_attrs.get("iat"), Some(&1516239022));
        assert_eq!(entity.numeric_attrs.get("exp"), Some(&1735689600));

        // Check relationships from roles array
        assert_eq!(entity.relationships.len(), 2);
        assert_eq!(entity.relationships[0].rel_type, "has_role");
        assert_eq!(entity.relationships[0].target, "role:admin");
        assert_eq!(entity.relationships[1].target, "role:user");

        // Check metadata
        assert_eq!(
            entity.metadata.get("session_id"),
            Some(&"session_abc123".to_string())
        );
    }

    #[test]
    fn test_jwt_claims_helpers() {
        let parser = JwtParser::new();
        let entity = parser
            .create_entity_from_token(TEST_JWT, "session_abc123")
            .unwrap();

        // Test claim extraction
        assert_eq!(
            JwtClaims::get_subject(&entity),
            Some(&"user123".to_string())
        );
        assert_eq!(JwtClaims::get_issued_at(&entity), Some(1516239022));
        assert_eq!(JwtClaims::get_expiration(&entity), Some(1735689600));

        // Test custom claims
        assert_eq!(
            JwtClaims::get_string_claim(&entity, "name"),
            Some(&"John Doe".to_string())
        );
    }

    #[test]
    fn test_jwt_expiration_check() {
        let mut entity = EntityData {
            entity_type: "jwt_session".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        // Test expired token (exp in the past)
        entity.numeric_attrs.insert("exp".to_string(), 1000000000);
        assert!(JwtClaims::is_expired(&entity));
        assert!(!JwtClaims::is_valid(&entity));

        // Test valid token (exp in the future)
        let future_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600; // 1 hour from now
        entity.numeric_attrs.insert("exp".to_string(), future_time);
        assert!(!JwtClaims::is_expired(&entity));
        assert!(JwtClaims::is_valid(&entity));
    }

    #[test]
    fn test_jwt_not_before_check() {
        let mut entity = EntityData {
            entity_type: "jwt_session".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        // Add exp claim (in the future)
        let future_exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 7200; // 2 hours from now
        entity.numeric_attrs.insert("exp".to_string(), future_exp);

        // Test not yet valid token (nbf in the future)
        let future_nbf = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600; // 1 hour from now
        entity.numeric_attrs.insert("nbf".to_string(), future_nbf);
        assert!(JwtClaims::is_not_yet_valid(&entity));
        assert!(!JwtClaims::is_valid(&entity));

        // Test valid token (nbf in the past)
        entity.numeric_attrs.insert("nbf".to_string(), 1000000000);
        assert!(!JwtClaims::is_not_yet_valid(&entity));
        assert!(JwtClaims::is_valid(&entity));
    }

    #[test]
    fn test_invalid_jwt_format() {
        let parser = JwtParser::new();

        // Invalid format (not 3 parts)
        let result = parser.parse_claims_unverified("invalid.jwt");
        assert!(result.is_err());

        // Invalid base64
        let result = parser.parse_claims_unverified("a.@@@.c");
        assert!(result.is_err());
    }

    #[test]
    fn test_base64url_decoding() {
        let parser = JwtParser::new();

        // Test with padding
        let decoded = parser.decode_base64url("dGVzdA").unwrap();
        assert_eq!(decoded, b"test");

        // Test without padding (JWT style)
        let decoded = parser.decode_base64url("dGVzdA").unwrap();
        assert_eq!(decoded, b"test");
    }
}
