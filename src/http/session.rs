use std::collections::HashMap;
use std::time::{Instant, Duration};

/// Session data: arbitrary key→value map
pub type SessionData = HashMap<String, String>;

struct Session {
    data: SessionData,
    last_access: Instant,
}

/// In-memory session store
pub struct SessionStore {
    sessions: HashMap<String, Session>,
    ttl: Duration,
}

impl SessionStore {
    pub fn new() -> Self {
        SessionStore {
            sessions: HashMap::new(),
            ttl: Duration::from_secs(1800), // 30 minutes default
        }
    }

    /// Create a new session, return its ID
    pub fn create(&mut self) -> String {
        let id = generate_session_id();
        self.sessions.insert(id.clone(), Session {
            data: HashMap::new(),
            last_access: Instant::now(),
        });
        id
    }

    /// Get session data (mutable), returns None if session expired or not found
    pub fn get_mut(&mut self, id: &str) -> Option<&mut SessionData> {
        let ttl = self.ttl;
        // Check expiry first, remove if expired
        if let Some(session) = self.sessions.get(id) {
            if session.last_access.elapsed() > ttl {
                self.sessions.remove(id);
                return None;
            }
        }
        let session = self.sessions.get_mut(id)?;
        session.last_access = Instant::now();
        Some(&mut session.data)
    }

    /// Parse session ID from Cookie header
    pub fn parse_session_id(cookie_header: &str) -> Option<String> {
        for part in cookie_header.split(';') {
            let part = part.trim();
            if let Some(val) = part.strip_prefix("session_id=") {
                return Some(val.trim().to_string());
            }
        }
        None
    }

    /// Remove expired sessions (call periodically)
    pub fn cleanup(&mut self) {
        let ttl = self.ttl;
        self.sessions.retain(|_, s| s.last_access.elapsed() <= ttl);
    }

    /// Parse cookies from a Cookie header into a map
    pub fn parse_cookies(cookie_header: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for part in cookie_header.split(';') {
            let part = part.trim();
            if let Some(eq) = part.find('=') {
                let key = part[..eq].trim().to_string();
                let val = part[eq+1..].trim().to_string();
                map.insert(key, val);
            }
        }
        map
    }
}

/// Generate a pseudorandom session ID using system time + counter
fn generate_session_id() -> String {
    use std::time::SystemTime;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Mix timestamp and counter for uniqueness
    let raw = ts ^ ((count as u128) * 0x9e3779b97f4a7c15);
    format!("{:032x}", raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_retrieve_session() {
        let mut store = SessionStore::new();
        let id = store.create();
        assert!(!id.is_empty());
        let data = store.get_mut(&id);
        assert!(data.is_some());
    }

    #[test]
    fn test_session_data_storage() {
        let mut store = SessionStore::new();
        let id = store.create();
        {
            let data = store.get_mut(&id).unwrap();
            data.insert("user".to_string(), "alice".to_string());
        }
        let data = store.get_mut(&id).unwrap();
        assert_eq!(data.get("user"), Some(&"alice".to_string()));
    }

    #[test]
    fn test_parse_session_id_from_cookie() {
        let cookie = "other=val; session_id=abc123; foo=bar";
        let id = SessionStore::parse_session_id(cookie);
        assert_eq!(id, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_cookies() {
        let header = "name=John; age=30; token=xyz";
        let cookies = SessionStore::parse_cookies(header);
        assert_eq!(cookies.get("name"), Some(&"John".to_string()));
        assert_eq!(cookies.get("age"), Some(&"30".to_string()));
        assert_eq!(cookies.get("token"), Some(&"xyz".to_string()));
    }

    #[test]
    fn test_session_ids_unique() {
        let mut store = SessionStore::new();
        let id1 = store.create();
        let id2 = store.create();
        assert_ne!(id1, id2);
    }
}
