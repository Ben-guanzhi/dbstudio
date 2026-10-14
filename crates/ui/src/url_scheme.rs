use std::collections::HashMap;
use url::Url;

/// Parsed dbstudio:// URL.
#[derive(Debug, Clone)]
pub struct DbStudioUrl {
    pub action: String,
    pub params: HashMap<String, String>,
}

impl DbStudioUrl {
    /// Parse a dbstudio:// URL from a string.
    pub fn parse(url_str: &str) -> Option<Self> {
        let url = Url::parse(url_str).ok()?;
        if url.scheme() != "dbstudio" {
            return None;
        }

        let action = url.host_str()?.to_string();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();

        Some(Self { action, params })
    }

    /// Check if this is a connect action.
    pub fn is_connect(&self) -> bool {
        self.action == "connect"
    }

    /// Get database type from params.
    pub fn db_type(&self) -> Option<&str> {
        self.params.get("type").map(|s| s.as_str())
    }

    /// Get host from params.
    pub fn host(&self) -> Option<&str> {
        self.params.get("host").map(|s| s.as_str())
    }

    /// Get port from params.
    pub fn port(&self) -> Option<u16> {
        self.params.get("port").and_then(|s| s.parse().ok())
    }

    /// Get database name from params.
    pub fn database(&self) -> Option<&str> {
        self.params.get("database").map(|s| s.as_str())
    }

    /// Get username from params.
    pub fn username(&self) -> Option<&str> {
        self.params.get("username").map(|s| s.as_str())
    }

    /// Get password from params.
    pub fn password(&self) -> Option<&str> {
        self.params.get("password").map(|s| s.as_str())
    }
}

/// Check command-line arguments for a dbstudio:// URL.
pub fn find_url_in_args() -> Option<String> {
    std::env::args().find(|arg| arg.starts_with("dbstudio://"))
}

// Inline tests extracted to url_scheme_tests.rs.
#[cfg(test)]
#[path = "url_scheme_tests.rs"]
mod url_scheme_tests;
