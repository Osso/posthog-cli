use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;

pub struct PostHogClient {
    client: Client,
    host: String,
    api_key: String,
    pub project_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub id: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FeatureFlag {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub active: bool,
    pub deleted: bool,
    pub filters: Value,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
struct LocalEvalFlag {
    id: i64,
    key: String,
    name: String,
    active: bool,
    deleted: bool,
    filters: Value,
    team_id: i64,
}

#[derive(Debug, Deserialize)]
struct LocalEvalResponse {
    flags: Vec<LocalEvalFlag>,
}

#[derive(Debug, Deserialize)]
struct PaginatedResponse<T> {
    results: Vec<T>,
}

fn local_eval_flag_to_feature_flag(f: LocalEvalFlag) -> FeatureFlag {
    FeatureFlag {
        id: f.id,
        key: f.key,
        name: f.name,
        active: f.active,
        deleted: f.deleted,
        filters: f.filters,
        created_at: String::new(),
    }
}

/// Bail with status and response body on non-2xx.
fn check_response(resp: Response, action: &str) -> Result<Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    bail!("Failed to {action}: HTTP {status} - {body}")
}

impl PostHogClient {
    pub fn new(config: &Config) -> Result<Self> {
        let client = Client::builder()
            .build()
            .context("Failed to build HTTP client")?;

        let mut ph = PostHogClient {
            client,
            host: config.host.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            project_id: 0,
        };

        ph.project_id = match config.project_id {
            Some(id) => id,
            None => ph.discover_project_id()?,
        };
        Ok(ph)
    }

    /// Authenticated GET. Returns `Ok(None)` on non-2xx for fallback patterns.
    fn try_get<T: DeserializeOwned>(&self, url: &str) -> Result<Option<T>> {
        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .context("Failed to connect to PostHog")?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        resp.json().map(Some).context("Failed to parse response")
    }

    fn discover_project_id(&self) -> Result<i64> {
        // Try local_evaluation first — works with feature_flag:read scope
        let url = format!("{}/api/feature_flag/local_evaluation/", self.host);
        if let Some(data) = self.try_get::<LocalEvalResponse>(&url)? {
            return data.flags.first().map(|f| f.team_id).context(
                "No feature flags found to determine project ID. Set project_id in config.",
            );
        }

        // Fall back to /api/projects/ (requires project:read scope)
        let url = format!("{}/api/projects/", self.host);
        let data: PaginatedResponse<Project> = self
            .try_get(&url)?
            .context("Failed to discover project ID. Add 'project_id = <id>' to your config.")?;
        data.results
            .into_iter()
            .next()
            .map(|p| p.id)
            .context("No projects found. Add 'project_id = <id>' to your config.")
    }

    pub fn list_flags(&self) -> Result<Vec<FeatureFlag>> {
        // Try management API first (full flag details with created_at)
        let url = format!(
            "{}/api/projects/{}/feature_flags/",
            self.host, self.project_id
        );
        if let Some(data) = self.try_get::<PaginatedResponse<FeatureFlag>>(&url)? {
            return Ok(data.results.into_iter().filter(|f| !f.deleted).collect());
        }

        // Fall back to local_evaluation endpoint (no created_at, but works with feature_flag:read)
        let url = format!("{}/api/feature_flag/local_evaluation/", self.host);
        let data: LocalEvalResponse = self
            .try_get(&url)?
            .context("Failed to list flags from both management and local evaluation APIs")?;
        Ok(data
            .flags
            .into_iter()
            .filter(|f| !f.deleted)
            .map(local_eval_flag_to_feature_flag)
            .collect())
    }

    pub fn get_flag_by_key(&self, key: &str) -> Result<FeatureFlag> {
        self.list_flags()?
            .into_iter()
            .find(|f| f.key == key)
            .with_context(|| format!("Feature flag '{key}' not found"))
    }

    pub fn create_flag(&self, key: &str) -> Result<FeatureFlag> {
        let url = format!(
            "{}/api/projects/{}/feature_flags/",
            self.host, self.project_id
        );
        let body = json!({
            "key": key,
            "name": key,
            "active": true,
            "filters": {
                "groups": [{ "properties": [], "rollout_percentage": 100 }]
            }
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .context("Failed to create feature flag")?;
        check_response(resp, "create flag")?
            .json()
            .context("Failed to parse create flag response")
    }

    pub fn set_flag_active(&self, key: &str, active: bool) -> Result<FeatureFlag> {
        let flag = self.get_flag_by_key(key)?;
        let url = format!(
            "{}/api/projects/{}/feature_flags/{}/",
            self.host, self.project_id, flag.id
        );

        let resp = self
            .client
            .patch(&url)
            .bearer_auth(&self.api_key)
            .json(&json!({ "active": active }))
            .send()
            .context("Failed to update feature flag")?;
        check_response(resp, "update flag")?
            .json()
            .context("Failed to parse update flag response")
    }

    pub fn delete_flag(&self, key: &str) -> Result<()> {
        let flag = self.get_flag_by_key(key)?;
        let url = format!(
            "{}/api/projects/{}/feature_flags/{}/",
            self.host, self.project_id, flag.id
        );

        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.api_key)
            .send()
            .context("Failed to delete feature flag")?;
        check_response(resp, "delete flag")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct MockPostHog {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl MockPostHog {
        fn start(handler: fn(&str) -> (&'static str, &'static str)) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);

            thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = stream.unwrap();
                    let mut buffer = [0_u8; 8192];
                    let bytes_read = stream.read(&mut buffer).unwrap();
                    let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                    let request_line = request.lines().next().unwrap_or_default();
                    let (status, body) = handler(request_line);
                    server_requests.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                }
            });

            Self {
                base_url: format!("http://{}", display_addr(addr)),
                requests,
            }
        }

        fn config(&self, project_id: Option<i64>) -> Config {
            Config {
                api_key: "phx_test".to_string(),
                host: format!("{}/", self.base_url),
                project_id,
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn display_addr(addr: SocketAddr) -> String {
        format!("{}:{}", addr.ip(), addr.port())
    }

    const LOCAL_EVAL_FLAGS: &str = r#"{"flags":[{"id":21,"key":"beta","name":"Beta","active":true,"deleted":false,"filters":{"groups":[{"rollout_percentage":50}]},"team_id":42},{"id":22,"key":"gone","name":"Gone","active":false,"deleted":true,"filters":{},"team_id":42}]}"#;

    fn management_flags_handler(request_line: &str) -> (&'static str, &'static str) {
        if request_line.starts_with("GET /api/projects/7/feature_flags/") {
            return (
                "200 OK",
                r#"{"results":[{"id":11,"key":"alpha","name":"Alpha","active":true,"deleted":false,"filters":{"groups":[]},"created_at":"2026-01-02T03:04:05Z"},{"id":12,"key":"deleted","name":"Deleted","active":false,"deleted":true,"filters":{},"created_at":"2026-01-03T00:00:00Z"}]}"#,
            );
        }
        ("404 Not Found", r#"{"detail":"missing"}"#)
    }

    fn local_eval_handler(request_line: &str) -> (&'static str, &'static str) {
        if request_line.starts_with("GET /api/projects/7/feature_flags/") {
            return ("403 Forbidden", r#"{"detail":"forbidden"}"#);
        }
        if request_line.starts_with("GET /api/feature_flag/local_evaluation/") {
            return ("200 OK", LOCAL_EVAL_FLAGS);
        }
        ("404 Not Found", r#"{"detail":"missing"}"#)
    }

    fn discovery_projects_handler(request_line: &str) -> (&'static str, &'static str) {
        if request_line.starts_with("GET /api/feature_flag/local_evaluation/") {
            return ("403 Forbidden", r#"{"detail":"forbidden"}"#);
        }
        if request_line.starts_with("GET /api/projects/ ") {
            return ("200 OK", r#"{"results":[{"id":99,"name":"Demo"}]}"#);
        }
        ("404 Not Found", r#"{"detail":"missing"}"#)
    }

    fn mutation_handler(request_line: &str) -> (&'static str, &'static str) {
        if request_line.starts_with("GET /api/projects/7/feature_flags/") {
            return (
                "200 OK",
                r#"{"results":[{"id":11,"key":"alpha","name":"Alpha","active":false,"deleted":false,"filters":{"groups":[]},"created_at":"2026-01-02T03:04:05Z"}]}"#,
            );
        }
        if request_line.starts_with("POST /api/projects/7/feature_flags/") {
            return (
                "201 Created",
                r#"{"id":13,"key":"created","name":"created","active":true,"deleted":false,"filters":{"groups":[]},"created_at":"2026-01-04T00:00:00Z"}"#,
            );
        }
        if request_line.starts_with("PATCH /api/projects/7/feature_flags/11/") {
            return (
                "200 OK",
                r#"{"id":11,"key":"alpha","name":"Alpha","active":true,"deleted":false,"filters":{"groups":[]},"created_at":"2026-01-02T03:04:05Z"}"#,
            );
        }
        if request_line.starts_with("DELETE /api/projects/7/feature_flags/11/") {
            return ("204 No Content", "");
        }
        ("404 Not Found", r#"{"detail":"missing"}"#)
    }

    fn error_handler(request_line: &str) -> (&'static str, &'static str) {
        if request_line.starts_with("POST /api/projects/7/feature_flags/") {
            return ("500 Internal Server Error", r#"{"detail":"boom"}"#);
        }
        ("404 Not Found", r#"{"detail":"missing"}"#)
    }

    #[test]
    fn new_uses_configured_project_and_trims_host() {
        let server = MockPostHog::start(management_flags_handler);
        let client = PostHogClient::new(&server.config(Some(7))).unwrap();

        assert_eq!(client.project_id, 7);
        assert_eq!(client.host, server.base_url);
    }

    #[test]
    fn discovers_project_id_from_local_evaluation_or_projects() {
        let local_server = MockPostHog::start(local_eval_handler);
        let local_client = PostHogClient::new(&local_server.config(None)).unwrap();
        assert_eq!(local_client.project_id, 42);

        let project_server = MockPostHog::start(discovery_projects_handler);
        let project_client = PostHogClient::new(&project_server.config(None)).unwrap();
        assert_eq!(project_client.project_id, 99);
    }

    #[test]
    fn list_flags_prefers_management_api_and_filters_deleted() {
        let server = MockPostHog::start(management_flags_handler);
        let client = PostHogClient::new(&server.config(Some(7))).unwrap();

        let flags = client.list_flags().unwrap();

        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].key, "alpha");
        assert_eq!(flags[0].created_at, "2026-01-02T03:04:05Z");
    }

    #[test]
    fn list_flags_falls_back_to_local_evaluation() {
        let server = MockPostHog::start(local_eval_handler);
        let client = PostHogClient::new(&server.config(Some(7))).unwrap();

        let flags = client.list_flags().unwrap();

        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].key, "beta");
        assert_eq!(flags[0].created_at, "");
    }

    #[test]
    fn get_flag_by_key_returns_match_or_contextual_error() {
        let server = MockPostHog::start(management_flags_handler);
        let client = PostHogClient::new(&server.config(Some(7))).unwrap();

        assert_eq!(client.get_flag_by_key("alpha").unwrap().id, 11);
        let err = client.get_flag_by_key("missing").unwrap_err();
        assert!(err.to_string().contains("Feature flag 'missing' not found"));
    }

    #[test]
    fn create_update_and_delete_use_expected_methods_and_auth() {
        let server = MockPostHog::start(mutation_handler);
        let client = PostHogClient::new(&server.config(Some(7))).unwrap();

        assert_eq!(client.create_flag("created").unwrap().id, 13);
        assert!(client.set_flag_active("alpha", true).unwrap().active);
        client.delete_flag("alpha").unwrap();

        let requests = server.requests();
        assert!(requests.iter().any(|request| {
            request.starts_with("POST /api/projects/7/feature_flags/")
                && request.contains("authorization: Bearer phx_test")
                && request.contains(r#""rollout_percentage":100"#)
        }));
        assert!(requests.iter().any(|request| {
            request.starts_with("PATCH /api/projects/7/feature_flags/11/")
                && request.contains(r#""active":true"#)
        }));
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("DELETE /api/projects/7/feature_flags/11/"))
        );
    }

    #[test]
    fn check_response_includes_status_and_body() {
        let server = MockPostHog::start(error_handler);
        let client = PostHogClient::new(&server.config(Some(7))).unwrap();

        let err = client.create_flag("created").unwrap_err();

        assert!(err.to_string().contains("HTTP 500 Internal Server Error"));
        assert!(err.to_string().contains("boom"));
    }
}
