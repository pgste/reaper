//! Password hashing and token utilities

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::Rng;
use sha2::{Digest, Sha256};

/// Hash a password using Argon2id
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())
}

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

/// Generate a session token (rst_ prefix for "reaper session token")
pub fn generate_session_token() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    format!("rst_{}", hex::encode(random_bytes))
}

/// Generate a password reset token
pub fn generate_reset_token() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    hex::encode(random_bytes)
}

/// Generate an email verification token (shorter for email-friendly URLs)
pub fn generate_verification_token() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..24).map(|_| rng.gen()).collect();
    hex::encode(random_bytes)
}

/// Hash a token for storage
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing() {
        let password = "SecurePass123!";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong_password", &hash));
    }

    #[test]
    fn test_session_token_generation() {
        let token = generate_session_token();
        assert!(token.starts_with("rst_"));
        assert_eq!(token.len(), 68); // "rst_" + 64 hex chars
    }

    #[test]
    fn test_token_hashing() {
        let token = "test_token";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);
        assert_eq!(hash1, hash2);
        assert_ne!(hash_token("different_token"), hash1);
    }
}
