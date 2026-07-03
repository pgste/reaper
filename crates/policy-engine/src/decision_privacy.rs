//! Decision-log data protection: masking, pseudonymization, encryption.
//!
//! Decision logs carry identity and, with the explain tier, resolved entity
//! attributes (clearance levels, departments, …). This module protects that
//! data **once, at capture** — inside `DecisionBuffer::log()` — so every
//! downstream view (query API ring, file/stdout sinks, NDJSON export, the
//! central pipeline) only ever sees protected values. Nothing here runs when
//! no protection is configured, and none of it runs on the eval path (only for
//! decisions that already passed the sampling gate).
//!
//! Three independent layers, composable per deployment:
//!
//! 1. **Principal pseudonymization** — `principal` is replaced with
//!    `sha256:<hex>` via HMAC-SHA-256 under a secret salt. Stable across
//!    entries (an investigator can still join "this same user did X then Y")
//!    but not reversible, and the keyed construction prevents dictionary
//!    attacks by anyone who has the logs but not the salt.
//! 2. **Context/attribute masking** — an optional allowlist drops all context
//!    keys not explicitly permitted, and `mask_keys` replaces the values of
//!    named keys (case-insensitive) with `"***"` in both the request context
//!    and the explain-tier `input_data` attribute maps.
//! 3. **`input_data` encryption at rest** — the explain snapshot is sealed
//!    with AES-256-GCM into an envelope only the key holder can open (the
//!    control plane holds per-tenant keys; a log-store operator sees only
//!    ciphertext). Fail-closed: enabling encryption without a valid 32-byte
//!    hex key is a configuration error, never a silent plaintext fallback.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::decision_log::{DecisionLogConfig, DecisionLogEntry};

type HmacSha256 = Hmac<Sha256>;

/// Replacement for masked values.
const MASKED: &str = "***";

/// Algorithm tag stored in the encryption envelope.
const ENC_ALG: &str = "aes256gcm";

/// Capture-time data protection, built once from config and applied to every
/// logged entry. See module docs.
pub struct DataProtection {
    /// HMAC key for principal pseudonymization (None = no hashing).
    hash_salt: Option<Vec<u8>>,
    /// Lowercased allowlist for context keys (None = keep all).
    context_allowlist: Option<Vec<String>>,
    /// Lowercased keys whose values are replaced with `"***"`.
    mask_keys: Vec<String>,
    /// AES-256-GCM cipher for `input_data` (None = no encryption).
    cipher: Option<Aes256Gcm>,
}

impl DataProtection {
    /// Build from config. Returns `Ok(None)` when no protection is configured
    /// (the buffer then skips the whole layer). Fails closed on invalid
    /// combinations: hashing without a salt, or encryption without a valid
    /// 32-byte hex key, is a configuration error — never a silent downgrade.
    pub fn from_config(config: &DecisionLogConfig) -> Result<Option<Self>, String> {
        let hash_salt = if config.hash_principal {
            match config.hash_salt.as_deref() {
                Some(salt) if !salt.is_empty() => Some(salt.as_bytes().to_vec()),
                _ => {
                    return Err("hash_principal requires a non-empty hash_salt \
                         (REAPER_DECISION_LOG_HASH_SALT); refusing unsalted hashing"
                        .to_string())
                }
            }
        } else {
            None
        };

        let cipher = if config.encrypt_input_data {
            let key_hex = config.encryption_key.as_deref().unwrap_or_default();
            let key_bytes = hex::decode(key_hex)
                .map_err(|_| "encryption_key must be hex (REAPER_DECISION_LOG_ENCRYPTION_KEY)")?;
            if key_bytes.len() != 32 {
                return Err(format!(
                    "encryption_key must be 32 bytes (64 hex chars), got {} bytes",
                    key_bytes.len()
                ));
            }
            Some(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes)))
        } else {
            None
        };

        let context_allowlist = config
            .context_allowlist
            .as_ref()
            .map(|keys| keys.iter().map(|k| k.to_lowercase()).collect());
        let mask_keys: Vec<String> = config.mask_keys.iter().map(|k| k.to_lowercase()).collect();

        if hash_salt.is_none()
            && cipher.is_none()
            && context_allowlist.is_none()
            && mask_keys.is_empty()
        {
            return Ok(None);
        }

        Ok(Some(Self {
            hash_salt,
            context_allowlist,
            mask_keys,
            cipher,
        }))
    }

    /// Apply all configured protection to an entry, in place. Runs on the
    /// capture path for decisions that passed sampling — cheap unless
    /// encryption is on (and that is opt-in, explain-tier-only data).
    /// Returns `Err` only if encryption itself fails, in which case the caller
    /// must NOT log the plaintext (fail closed).
    pub fn apply(&self, entry: &mut DecisionLogEntry) -> Result<(), String> {
        if let Some(ref salt) = self.hash_salt {
            entry.principal = pseudonymize(salt, &entry.principal);
        }

        // Context: allowlist first, then masking.
        if let Some(ref allow) = self.context_allowlist {
            entry
                .context
                .retain(|k, _| allow.iter().any(|a| a == &k.to_lowercase()));
        }
        if !self.mask_keys.is_empty() {
            for (k, v) in entry.context.iter_mut() {
                if self.is_masked(k) {
                    *v = Value::String(MASKED.to_string());
                }
            }
            if let Some(ref mut input) = entry.input_data {
                mask_input_data(input, |k| self.is_masked(k));
            }
        }

        // Encryption last, over the already-masked snapshot.
        if let Some(ref cipher) = self.cipher {
            if let Some(input) = entry.input_data.take() {
                entry.input_data = Some(encrypt_value(cipher, &input)?);
            }
        }

        Ok(())
    }

    fn is_masked(&self, key: &str) -> bool {
        let k = key.to_lowercase();
        self.mask_keys.iter().any(|m| m == &k)
    }
}

/// `sha256:<hex>` HMAC-SHA-256 pseudonym — stable, irreversible, keyed.
pub fn pseudonymize(salt: &[u8], value: &str) -> String {
    // Fully qualified: both `hmac::Mac` and the AES `KeyInit` traits provide a
    // `new_from_slice` in this scope.
    let mut mac = <HmacSha256 as Mac>::new_from_slice(salt).expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    let digest = mac.finalize().into_bytes();
    // 128 bits is ample for joinability without collisions; keeps lines short.
    format!("sha256:{}", hex::encode(&digest[..16]))
}

/// Mask matching attribute keys inside the explain snapshot
/// (`{"principal": {..attrs..}, "resource": {..attrs..}}`).
fn mask_input_data<F: Fn(&str) -> bool>(input: &mut Value, is_masked: F) {
    if let Value::Object(sections) = input {
        for section in sections.values_mut() {
            if let Value::Object(attrs) = section {
                for (k, v) in attrs.iter_mut() {
                    if is_masked(k) {
                        *v = Value::String(MASKED.to_string());
                    }
                }
            }
        }
    }
}

/// Seal a JSON value into an AES-256-GCM envelope:
/// `{"enc":"aes256gcm","nonce":"<b64>","ciphertext":"<b64>"}`.
fn encrypt_value(cipher: &Aes256Gcm, value: &Value) -> Result<Value, String> {
    let plaintext = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_slice())
        .map_err(|e| format!("input_data encryption failed: {e}"))?;
    Ok(serde_json::json!({
        "enc": ENC_ALG,
        "nonce": B64.encode(nonce),
        "ciphertext": B64.encode(ciphertext),
    }))
}

/// Open an `input_data` encryption envelope with the 32-byte hex key. Used by
/// the control plane / tooling; the agent never decrypts.
pub fn decrypt_input_data(envelope: &Value, key_hex: &str) -> Result<Value, String> {
    let obj = envelope
        .as_object()
        .ok_or("envelope must be a JSON object")?;
    let alg = obj.get("enc").and_then(Value::as_str).unwrap_or_default();
    if alg != ENC_ALG {
        return Err(format!("unsupported envelope algorithm: {alg:?}"));
    }
    let nonce_bytes = B64
        .decode(obj.get("nonce").and_then(Value::as_str).unwrap_or_default())
        .map_err(|e| format!("bad nonce: {e}"))?;
    let ciphertext = B64
        .decode(
            obj.get("ciphertext")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .map_err(|e| format!("bad ciphertext: {e}"))?;

    let key_bytes = hex::decode(key_hex).map_err(|_| "key must be hex")?;
    if key_bytes.len() != 32 {
        return Err("key must be 32 bytes (64 hex chars)".to_string());
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    if nonce_bytes.len() != 12 {
        return Err("nonce must be 12 bytes".to_string());
    }
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_slice())
        .map_err(|_| "decryption failed (wrong key or tampered data)".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|e| e.to_string())
}

/// Generate a fresh random 32-byte AES-256-GCM key, hex-encoded — for
/// `reaper-cli keygen`-style tooling.
pub fn generate_encryption_key_hex() -> String {
    let key = Aes256Gcm::generate_key(&mut OsRng);
    hex::encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn entry_with_context() -> DecisionLogEntry {
        let mut context = HashMap::new();
        context.insert("ip".to_string(), json!("10.0.0.1"));
        context.insert("session_token".to_string(), json!("secret-token-xyz"));
        context.insert("request_id".to_string(), json!("req-1"));
        DecisionLogEntry::new(
            "alice@example.com".to_string(),
            "read".to_string(),
            "/records/42".to_string(),
            "deny".to_string(),
            "pol-1".to_string(),
            "records-policy".to_string(),
        )
        .with_context(context)
        .with_input_data(json!({
            "principal": {"role": "nurse", "ssn": "123-45-6789", "clearance_level": 2},
            "resource": {"department": "oncology", "clearance_level": 4}
        }))
    }

    fn protection(config: &DecisionLogConfig) -> DataProtection {
        DataProtection::from_config(config)
            .expect("valid config")
            .expect("protection configured")
    }

    #[test]
    fn no_protection_configured_returns_none() {
        let config = DecisionLogConfig::default();
        assert!(DataProtection::from_config(&config).unwrap().is_none());
    }

    #[test]
    fn hash_principal_is_stable_keyed_and_irreversible_format() {
        let config = DecisionLogConfig {
            hash_principal: true,
            hash_salt: Some("tenant-secret".to_string()),
            ..Default::default()
        };
        let p = protection(&config);

        let mut e1 = entry_with_context();
        let mut e2 = entry_with_context();
        p.apply(&mut e1).unwrap();
        p.apply(&mut e2).unwrap();

        assert!(e1.principal.starts_with("sha256:"), "{}", e1.principal);
        assert!(
            !e1.principal.contains("alice"),
            "raw identity must not leak"
        );
        assert_eq!(e1.principal, e2.principal, "stable → joinable");

        // Different salt → different pseudonym (keyed, not a plain hash).
        let other = DecisionLogConfig {
            hash_salt: Some("other-secret".to_string()),
            ..config
        };
        let mut e3 = entry_with_context();
        protection(&other).apply(&mut e3).unwrap();
        assert_ne!(e1.principal, e3.principal);
    }

    #[test]
    fn hash_principal_without_salt_fails_closed() {
        let config = DecisionLogConfig {
            hash_principal: true,
            hash_salt: None,
            ..Default::default()
        };
        assert!(DataProtection::from_config(&config).is_err());
    }

    #[test]
    fn context_allowlist_drops_unlisted_keys() {
        let config = DecisionLogConfig {
            context_allowlist: Some(vec!["request_id".to_string(), "IP".to_string()]),
            ..Default::default()
        };
        let mut e = entry_with_context();
        protection(&config).apply(&mut e).unwrap();

        assert_eq!(e.context.len(), 2);
        assert!(
            e.context.contains_key("ip"),
            "allowlist is case-insensitive"
        );
        assert!(e.context.contains_key("request_id"));
        assert!(!e.context.contains_key("session_token"));
    }

    #[test]
    fn mask_keys_mask_context_and_input_data() {
        let config = DecisionLogConfig {
            mask_keys: vec!["Session_Token".to_string(), "SSN".to_string()],
            ..Default::default()
        };
        let mut e = entry_with_context();
        protection(&config).apply(&mut e).unwrap();

        assert_eq!(e.context["session_token"], json!("***"));
        assert_eq!(
            e.context["ip"],
            json!("10.0.0.1"),
            "unlisted keys untouched"
        );

        let input = e.input_data.unwrap();
        assert_eq!(input["principal"]["ssn"], json!("***"));
        assert_eq!(input["principal"]["role"], json!("nurse"));
        assert_eq!(input["resource"]["department"], json!("oncology"));
    }

    #[test]
    fn encrypt_input_data_round_trips_and_hides_plaintext() {
        let key = generate_encryption_key_hex();
        let config = DecisionLogConfig {
            encrypt_input_data: true,
            encryption_key: Some(key.clone()),
            ..Default::default()
        };
        let mut e = entry_with_context();
        protection(&config).apply(&mut e).unwrap();

        let envelope = e.input_data.clone().unwrap();
        assert_eq!(envelope["enc"], json!("aes256gcm"));
        let as_text = envelope.to_string();
        assert!(!as_text.contains("oncology"), "no plaintext in envelope");
        assert!(!as_text.contains("123-45-6789"));

        // Key holder can open it.
        let opened = decrypt_input_data(&envelope, &key).unwrap();
        assert_eq!(opened["resource"]["department"], json!("oncology"));

        // Wrong key cannot.
        let wrong = generate_encryption_key_hex();
        assert!(decrypt_input_data(&envelope, &wrong).is_err());
    }

    #[test]
    fn masking_applies_before_encryption() {
        let key = generate_encryption_key_hex();
        let config = DecisionLogConfig {
            mask_keys: vec!["ssn".to_string()],
            encrypt_input_data: true,
            encryption_key: Some(key.clone()),
            ..Default::default()
        };
        let mut e = entry_with_context();
        protection(&config).apply(&mut e).unwrap();

        let opened = decrypt_input_data(&e.input_data.unwrap(), &key).unwrap();
        assert_eq!(
            opened["principal"]["ssn"],
            json!("***"),
            "even the key holder never sees masked fields"
        );
    }

    #[test]
    fn encrypt_without_key_fails_closed() {
        for bad_key in [None, Some(String::new()), Some("abcd".to_string())] {
            let config = DecisionLogConfig {
                encrypt_input_data: true,
                encryption_key: bad_key,
                ..Default::default()
            };
            assert!(
                DataProtection::from_config(&config).is_err(),
                "must refuse to start rather than log plaintext"
            );
        }
    }

    #[test]
    fn secrets_never_serialize_out_of_config() {
        let config = DecisionLogConfig {
            hash_principal: true,
            hash_salt: Some("super-secret-salt".to_string()),
            encrypt_input_data: true,
            encryption_key: Some(generate_encryption_key_hex()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("super-secret-salt"));
        assert!(!json.contains("hash_salt"));
        assert!(!json.contains("encryption_key"));
        assert!(
            json.contains("hash_principal"),
            "non-secret flags still echo"
        );
    }
}
