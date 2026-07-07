//! JWT structure interpretation for policies (OPA `io.jwt.decode` parity).
//!
//! `jwt::decode(token)` / `jwt::header(token)` parse a compact JWS
//! (`header.payload.signature`, base64url) into objects that policies
//! navigate like any other data — claims, scopes, issuer, expiry:
//!
//! ```reap
//! claims := jwt::decode(input.token) &&
//! claims.iss == "https://issuer.corp" &&
//! "payments:write" in scopes  // scopes := claims.scope.split(" ")
//! ```
//!
//! SECURITY: these decode WITHOUT signature verification — the same contract
//! as OPA's `io.jwt.decode`. Verify signatures at the trust boundary (the
//! gateway/agent TLS + JWKS layer) and treat the policy layer as interpreting
//! an already-authenticated artifact. Malformed tokens decode to `null`, so
//! rules over bad tokens fail closed instead of erroring (total evaluation).

use super::super::types::EvalValue;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reaper_core::ReaperError;

/// Decode the CLAIMS (payload) segment of a compact JWT into an object.
/// Returns Null for anything that is not a well-formed `a.b.c` token with
/// base64url-JSON segments.
pub fn decode(token: &str) -> Result<EvalValue, ReaperError> {
    Ok(decode_segment(token, 1).unwrap_or(EvalValue::Null))
}

/// Decode the HEADER segment (alg/kid/typ) of a compact JWT into an object.
pub fn header(token: &str) -> Result<EvalValue, ReaperError> {
    Ok(decode_segment(token, 0).unwrap_or(EvalValue::Null))
}

fn decode_segment(token: &str, index: usize) -> Option<EvalValue> {
    // Accept a raw compact JWS or an Authorization header value.
    let token = token.trim();
    let token = token.strip_prefix("Bearer ").unwrap_or(token);

    let mut parts = token.split('.');
    let segment = match index {
        0 => parts.next()?,
        _ => {
            parts.next()?;
            parts.next()?
        }
    };
    // A compact JWS has exactly 3 segments (signature may be empty for
    // alg=none artifacts, but the separators must be present).
    let rest: Vec<&str> = token.split('.').collect();
    if rest.len() != 3 {
        return None;
    }

    let bytes = URL_SAFE_NO_PAD.decode(segment.as_bytes()).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    super::json::json_to_eval_value(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(header: &str, payload: &str) -> String {
        format!(
            "{}.{}.sig-not-checked",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(payload)
        )
    }

    #[test]
    fn decodes_claims_and_header() {
        let token = make_token(
            r#"{"alg":"RS256","kid":"key-1"}"#,
            r#"{"sub":"alice","iss":"https://issuer.corp","scope":"read write","exp":4102444800}"#,
        );
        let claims = decode(&token).unwrap();
        let EvalValue::Object(map) = claims else {
            panic!("claims must be an object")
        };
        assert_eq!(map.get("sub"), Some(&EvalValue::String("alice".into())));
        assert_eq!(map.get("exp"), Some(&EvalValue::Integer(4102444800)));

        let EvalValue::Object(h) = header(&token).unwrap() else {
            panic!("header must be an object")
        };
        assert_eq!(h.get("kid"), Some(&EvalValue::String("key-1".into())));
    }

    #[test]
    fn accepts_bearer_prefix() {
        let token = format!(
            "Bearer {}",
            make_token(r#"{"alg":"none"}"#, r#"{"sub":"x"}"#)
        );
        assert!(matches!(decode(&token).unwrap(), EvalValue::Object(_)));
    }

    #[test]
    fn malformed_tokens_decode_to_null_not_error() {
        for bad in [
            "",
            "not-a-jwt",
            "a.b",
            "a.b.c.d",
            "!!!.###.$$$",
            "Bearer garbage",
        ] {
            assert!(
                matches!(decode(bad).unwrap(), EvalValue::Null),
                "{bad:?} must decode to Null"
            );
        }
    }
}
