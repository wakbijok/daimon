#[cfg(feature = "ssr")]
use rusqlite::{Connection, params};

#[cfg(feature = "ssr")]
pub fn init_db(path: &str) -> Connection {
    let conn = Connection::open(path).expect("Failed to open database");
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'admin',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
        CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    ").expect("Failed to create tables");
    conn
}

#[cfg(feature = "ssr")]
pub fn find_user(conn: &Connection, username: &str) -> Option<(i64, String, String)> {
    conn.query_row(
        "SELECT id, username, password_hash FROM users WHERE username = ?1",
        params![username],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn create_user(conn: &Connection, username: &str, password_hash: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
        params![username, password_hash],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(feature = "ssr")]
pub fn insert_session(conn: &Connection, id: &str, user_id: i64, expires_at: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![id, user_id, expires_at],
    )?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub fn find_valid_session(conn: &Connection, id: &str) -> Option<(String, i64, String)> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
    conn.query_row(
        "SELECT id, user_id, expires_at FROM sessions WHERE id = ?1 AND expires_at > ?2",
        params![id, now],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn delete_session(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub fn get_config(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM config WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn set_config(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn test_user_crud() {
        let conn = init_db(":memory:");
        let id = create_user(&conn, "admin", "$2b$12$hash").unwrap();
        assert!(id > 0);

        let (uid, username, hash) = find_user(&conn, "admin").unwrap();
        assert_eq!(uid, id);
        assert_eq!(username, "admin");
        assert_eq!(hash, "$2b$12$hash");

        assert!(find_user(&conn, "nonexistent").is_none());
    }

    #[test]
    fn test_session_crud() {
        let conn = init_db(":memory:");
        let user_id = create_user(&conn, "admin", "hash").unwrap();

        // Use a far-future timestamp so the session is valid
        let far_future = "9999999999";
        insert_session(&conn, "sess-123", user_id, far_future).unwrap();

        let (sid, uid, exp) = find_valid_session(&conn, "sess-123").unwrap();
        assert_eq!(sid, "sess-123");
        assert_eq!(uid, user_id);
        assert_eq!(exp, far_future);

        delete_session(&conn, "sess-123").unwrap();
        assert!(find_valid_session(&conn, "sess-123").is_none());
    }

    #[test]
    fn test_expired_session_not_found() {
        let conn = init_db(":memory:");
        let user_id = create_user(&conn, "admin", "hash").unwrap();

        // Use a past timestamp
        insert_session(&conn, "sess-old", user_id, "0").unwrap();
        assert!(find_valid_session(&conn, "sess-old").is_none());
    }

    #[test]
    fn test_config_crud() {
        let conn = init_db(":memory:");
        set_config(&conn, "jwt_secret", "mysecret").unwrap();
        assert_eq!(get_config(&conn, "jwt_secret").unwrap(), "mysecret");

        set_config(&conn, "jwt_secret", "newsecret").unwrap();
        assert_eq!(get_config(&conn, "jwt_secret").unwrap(), "newsecret");
    }
}
