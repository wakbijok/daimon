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
        CREATE TABLE IF NOT EXISTS clusters (
            id TEXT PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            api_url TEXT NOT NULL,
            token TEXT NOT NULL,
            notes TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS user_preferences (
            user_id INTEGER NOT NULL,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (user_id, key),
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
    ").expect("Failed to create tables");
    conn
}

#[cfg(feature = "ssr")]
pub fn find_user(conn: &Connection, username: &str) -> Option<(i64, String, String, String)> {
    conn.query_row(
        "SELECT id, username, password_hash, role FROM users WHERE username = ?1",
        params![username],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
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

// --- Cluster CRUD ---

#[cfg(feature = "ssr")]
pub fn list_clusters(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM clusters ORDER BY name")
        .expect("prepare list_clusters");
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query list_clusters")
        .filter_map(|r| r.ok())
        .collect()
}

#[cfg(feature = "ssr")]
pub fn get_cluster(conn: &Connection, id: &str) -> Option<(String, String, String, String, String, String)> {
    conn.query_row(
        "SELECT id, name, api_url, token, notes, created_at FROM clusters WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn insert_cluster(conn: &Connection, id: &str, name: &str, api_url: &str, token: &str, notes: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO clusters (id, name, api_url, token, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, api_url, token, notes],
    )?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub fn delete_cluster(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM clusters WHERE id = ?1", params![id])?;
    Ok(())
}

// --- User Preferences ---

#[cfg(feature = "ssr")]
pub fn get_preference(conn: &Connection, user_id: i64, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = ?2",
        params![user_id, key],
        |row| row.get(0),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn set_preference(conn: &Connection, user_id: i64, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO user_preferences (user_id, key, value) VALUES (?1, ?2, ?3)",
        params![user_id, key, value],
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

        let (uid, username, hash, role) = find_user(&conn, "admin").unwrap();
        assert_eq!(uid, id);
        assert_eq!(username, "admin");
        assert_eq!(hash, "$2b$12$hash");
        assert_eq!(role, "admin");

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

    #[test]
    fn test_cluster_crud() {
        let conn = init_db(":memory:");
        insert_cluster(&conn, "c1", "Lab", "https://pve:8006", "root@pam!t=abc", "").unwrap();

        let clusters = list_clusters(&conn);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].0, "c1");
        assert_eq!(clusters[0].1, "Lab");

        let (id, name, url, token, notes, _created) = get_cluster(&conn, "c1").unwrap();
        assert_eq!(id, "c1");
        assert_eq!(name, "Lab");
        assert_eq!(url, "https://pve:8006");
        assert_eq!(token, "root@pam!t=abc");
        assert_eq!(notes, "");

        delete_cluster(&conn, "c1").unwrap();
        assert!(get_cluster(&conn, "c1").is_none());
        assert!(list_clusters(&conn).is_empty());
    }

    #[test]
    fn test_cluster_name_unique() {
        let conn = init_db(":memory:");
        insert_cluster(&conn, "c1", "Lab", "https://pve:8006", "tok", "").unwrap();
        let res = insert_cluster(&conn, "c2", "Lab", "https://pve:8006", "tok", "");
        assert!(res.is_err());
    }

    #[test]
    fn test_preference_crud() {
        let conn = init_db(":memory:");
        let uid = create_user(&conn, "admin", "hash").unwrap();

        assert!(get_preference(&conn, uid, "theme").is_none());

        set_preference(&conn, uid, "theme", "dark").unwrap();
        assert_eq!(get_preference(&conn, uid, "theme").unwrap(), "dark");

        set_preference(&conn, uid, "theme", "light").unwrap();
        assert_eq!(get_preference(&conn, uid, "theme").unwrap(), "light");
    }

    #[test]
    fn test_find_user_returns_role() {
        let conn = init_db(":memory:");
        create_user(&conn, "admin", "hash").unwrap();
        let (_id, _name, _hash, role) = find_user(&conn, "admin").unwrap();
        assert_eq!(role, "admin");
    }
}
