#[cfg(feature = "ssr")]
use jsonwebtoken::{encode, decode, Header, Algorithm, Validation, EncodingKey, DecodingKey};
#[cfg(feature = "ssr")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub user_id: i64,
    pub exp: usize,
    pub session_id: String,
}

#[cfg(feature = "ssr")]
pub fn hash_password(password: &str) -> String {
    bcrypt::hash(password, 12).expect("Failed to hash password")
}

#[cfg(feature = "ssr")]
pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

#[cfg(feature = "ssr")]
pub fn create_jwt(secret: &str, username: &str, user_id: i64, session_id: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let claims = Claims {
        sub: username.to_string(),
        user_id,
        exp: (now + 86400) as usize, // 24 hours
        session_id: session_id.to_string(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    ).expect("Failed to create JWT")
}

#[cfg(feature = "ssr")]
pub fn validate_jwt(secret: &str, token: &str) -> Option<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .ok()
    .map(|data| data.claims)
}

#[cfg(feature = "ssr")]
pub fn generate_secret() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn test_password_hash_and_verify() {
        let hash = hash_password("mypassword");
        assert!(verify_password("mypassword", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn test_jwt_roundtrip() {
        let secret = "test-secret-key";
        let token = create_jwt(secret, "admin", 1, "sess-abc");
        let claims = validate_jwt(secret, &token).unwrap();
        assert_eq!(claims.sub, "admin");
        assert_eq!(claims.user_id, 1);
        assert_eq!(claims.session_id, "sess-abc");
    }

    #[test]
    fn test_jwt_invalid_secret() {
        let token = create_jwt("secret1", "admin", 1, "sess-abc");
        assert!(validate_jwt("secret2", &token).is_none());
    }

    #[test]
    fn test_generate_secret_is_unique() {
        let s1 = generate_secret();
        let s2 = generate_secret();
        assert_ne!(s1, s2);
        assert_eq!(s1.len(), 64); // 32 bytes = 64 hex chars
    }
}
