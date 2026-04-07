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
    pub name: String,
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
