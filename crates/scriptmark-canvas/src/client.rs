use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanvasError {
    #[error("CANVAS_TOKEN environment variable not set")]
    MissingToken,
    #[error("HTTP request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("Canvas API error ({status}): {message}")]
    ApiError { status: u16, message: String },
}

/// Canvas LMS API client.
pub struct CanvasClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasUser {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub sortable_name: Option<String>,
    #[serde(default)]
    pub sis_user_id: Option<String>,
    #[serde(default)]
    pub login_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasSubmission {
    pub id: u64,
    pub user_id: u64,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub grade: Option<String>,
}

impl CanvasClient {
    /// Create a new Canvas client.
    ///
    /// `base_url` should be like `https://canvas.university.edu`
    /// Token is read from `CANVAS_TOKEN` env var.
    pub fn new(base_url: &str) -> Result<Self, CanvasError> {
        let token = std::env::var("CANVAS_TOKEN").map_err(|_| CanvasError::MissingToken)?;
        let base_url = base_url.trim_end_matches('/').to_string();
        Ok(Self {
            base_url,
            token,
            client: reqwest::Client::new(),
        })
    }

    /// Create a client with an explicit token (for testing).
    pub fn with_token(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Pull the student roster for a course.
    ///
    /// Returns a map of student_id (SIS ID or login_id) -> student name.
    pub async fn pull_roster(
        &self,
        course_id: u64,
    ) -> Result<HashMap<String, String>, CanvasError> {
        let mut roster = HashMap::new();
        let mut page = 1u32;

        loop {
            let url = format!(
                "{}/api/v1/courses/{}/users?enrollment_type[]=student&per_page=100&page={}",
                self.base_url, course_id, page
            );

            let response = self
                .client
                .get(&url)
                .bearer_auth(&self.token)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(CanvasError::ApiError {
                    status,
                    message: body,
                });
            }

            let users: Vec<CanvasUser> = response.json().await?;

            if users.is_empty() {
                break;
            }

            for user in &users {
                // Prefer SIS user ID, fall back to login_id, then Canvas ID
                let student_id = user
                    .sis_user_id
                    .clone()
                    .or_else(|| user.login_id.clone())
                    .unwrap_or_else(|| user.id.to_string());
                roster.insert(student_id, user.name.clone());
            }

            page += 1;
        }

        Ok(roster)
    }

    /// Push grades to a Canvas assignment.
    ///
    /// `grades` maps student Canvas user ID -> score.
    pub async fn push_grades(
        &self,
        course_id: u64,
        assignment_id: u64,
        grades: &HashMap<u64, f64>,
    ) -> Result<Vec<CanvasSubmission>, CanvasError> {
        let mut results = Vec::new();

        for (user_id, score) in grades {
            let url = format!(
                "{}/api/v1/courses/{}/assignments/{}/submissions/{}",
                self.base_url, course_id, assignment_id, user_id
            );

            let body = serde_json::json!({
                "submission": {
                    "posted_grade": score.to_string()
                }
            });

            let response = self
                .client
                .put(&url)
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let msg = response.text().await.unwrap_or_default();
                eprintln!(
                    "Warning: failed to push grade for user {}: {} {}",
                    user_id, status, msg
                );
                continue;
            }

            let submission: CanvasSubmission = response.json().await?;
            results.push(submission);
        }

        Ok(results)
    }

    /// Save roster to a CSV file (compatible with ScriptMark's roster format).
    pub fn save_roster_csv(
        roster: &HashMap<String, String>,
        path: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        writeln!(f, "name,class,student_id")?;
        let mut entries: Vec<_> = roster.iter().collect();
        entries.sort_by_key(|(id, _)| (*id).clone());
        for (student_id, name) in entries {
            writeln!(f, "{},,{}", name, student_id)?;
        }
        Ok(())
    }
}
