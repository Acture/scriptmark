use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::input::canvas::{
	CanvasAssignmentPayload, CanvasCoursePayload, CanvasSubmissionPayload, CanvasUserPayload,
};

#[derive(Debug, Error)]
pub enum CanvasError {
	#[error("CANVAS_TOKEN environment variable not set")]
	MissingToken,
	#[error("HTTP request failed: {0}")]
	RequestError(#[from] reqwest::Error),
	#[error("Canvas API error ({status}): {message}")]
	ApiError { status: u16, message: String },
	#[error("pagination did not terminate: {0}")]
	PaginationError(String),
	#[error("could not write {path}: {source}")]
	IoError {
		path: std::path::PathBuf,
		#[source]
		source: std::io::Error,
	},
}

/// How long to wait for a connection, and for a whole request to finish.
///
/// `reqwest::Client::new()` leaves every timeout unset. A dead peer still surfaces through
/// TCP keepalive in about a minute, but a *stalled* hop — connection open, body never
/// completing — never errors at all, and at the default download concurrency of 1 that
/// blocks an entire class's import behind one file. The body budget is generous because a
/// large attachment over a slow link is legitimate; it is the stall this bounds.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// A backstop against a server that keeps handing back a `next` link.
const MAX_PAGES: usize = 1000;

/// Canvas LMS API client.
pub struct CanvasClient {
	base_url: String,
	token: String,
	client: reqwest::Client,
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

/// The `rel="next"` target of a `Link` header, or `None` when this is the last page.
///
/// Canvas's pagination links are documented as opaque bookmarks — "these links should be
/// treated as opaque" — so the URL is returned verbatim and never rebuilt. Terminating on
/// a short page instead would be wrong: the docs say the `per_page` ceiling is unspecified,
/// so a full page is not promised even mid-walk.
fn next_link(headers: &reqwest::header::HeaderMap) -> Option<String> {
	let header = headers.get(reqwest::header::LINK)?.to_str().ok()?;
	for part in header.split(',') {
		let mut fields = part.split(';');
		let Some(target) = fields.next() else {
			continue;
		};
		let target = target.trim();
		let Some(url) = target
			.strip_prefix('<')
			.and_then(|rest| rest.strip_suffix('>'))
		else {
			continue;
		};
		let is_next = fields.any(|field| {
			let field = field.trim();
			field == "rel=\"next\"" || field == "rel=next"
		});
		if is_next {
			return Some(url.to_string());
		}
	}
	None
}

impl CanvasClient {
	/// Create a new Canvas client.
	///
	/// `base_url` should be like `https://canvas.university.edu`
	/// Token is read from `CANVAS_TOKEN` env var.
	pub fn new(base_url: &str) -> Result<Self, CanvasError> {
		let token = std::env::var("CANVAS_TOKEN").map_err(|_| CanvasError::MissingToken)?;
		Ok(Self::with_token(base_url, &token))
	}

	/// Create a client with an explicit token (for testing).
	pub fn with_token(base_url: &str, token: &str) -> Self {
		Self {
			base_url: base_url.trim_end_matches('/').to_string(),
			token: token.to_string(),
			client: reqwest::Client::builder()
				.connect_timeout(CONNECT_TIMEOUT)
				.timeout(REQUEST_TIMEOUT)
				.build()
				.expect("a client with only timeouts set always builds"),
		}
	}

	async fn send(&self, url: &str) -> Result<reqwest::Response, CanvasError> {
		let response = self.client.get(url).bearer_auth(&self.token).send().await?;
		if !response.status().is_success() {
			let status = response.status().as_u16();
			let message = response.text().await.unwrap_or_default();
			return Err(CanvasError::ApiError { status, message });
		}
		Ok(response)
	}

	/// One object from one request.
	async fn get_one<T: DeserializeOwned>(&self, url: &str) -> Result<T, CanvasError> {
		Ok(self.send(url).await?.json().await?)
	}

	/// Every page of a listing, followed through the `Link` header.
	///
	/// A failure on *any* page is a hard error rather than a short list: these listings
	/// establish who is in the course, and a page that quietly went missing would become a
	/// cohort of students who look like they never submitted.
	async fn paginate<T: DeserializeOwned>(&self, url: &str) -> Result<Vec<T>, CanvasError> {
		let mut items: Vec<T> = Vec::new();
		let mut next = Some(url.to_string());
		let mut seen = 0usize;

		while let Some(url) = next {
			seen += 1;
			if seen > MAX_PAGES {
				return Err(CanvasError::PaginationError(format!(
					"stopped after {MAX_PAGES} pages; last url was {url}"
				)));
			}

			let response = self.send(&url).await?;
			let following = next_link(response.headers());
			items.extend(response.json::<Vec<T>>().await?);

			// A server echoing the URL just fetched would otherwise spin until MAX_PAGES.
			if following.as_deref() == Some(url.as_str()) {
				return Err(CanvasError::PaginationError(format!(
					"the next link points at the page just fetched: {url}"
				)));
			}
			next = following;
		}

		Ok(items)
	}

	/// Courses the token can see, so a teacher can find an id without leaving the terminal.
	pub async fn list_courses(&self) -> Result<Vec<CanvasCoursePayload>, CanvasError> {
		self.paginate(&format!(
			"{}/api/v1/courses?enrollment_type=teacher&include[]=term&per_page=100",
			self.base_url
		))
		.await
	}

	pub async fn list_assignments(
		&self,
		course_id: u64,
	) -> Result<Vec<CanvasAssignmentPayload>, CanvasError> {
		self.paginate(&format!(
			"{}/api/v1/courses/{course_id}/assignments?per_page=100",
			self.base_url
		))
		.await
	}

	pub async fn fetch_assignment(
		&self,
		course_id: u64,
		assignment_id: u64,
	) -> Result<CanvasAssignmentPayload, CanvasError> {
		self.get_one(&format!(
			"{}/api/v1/courses/{course_id}/assignments/{assignment_id}",
			self.base_url
		))
		.await
	}

	/// Pull the student roster for a course.
	///
	/// `enrollment_state[]` is stated rather than inherited: the default omits `completed`,
	/// so a student who dropped after submitting would be missing here while present in the
	/// submissions listing — arriving as an unknown user with no student number instead of
	/// an ordinary enrollee.
	pub async fn pull_roster(&self, course_id: u64) -> Result<Vec<CanvasUserPayload>, CanvasError> {
		self.paginate(&format!(
			"{}/api/v1/courses/{course_id}/users\
			 ?enrollment_type[]=student&enrollment_state[]=active&enrollment_state[]=completed\
			 &per_page=100",
			self.base_url
		))
		.await
	}

	/// Every submission for an assignment, including earlier attempts.
	///
	/// `include[]=submission_history` is not optional. Canvas gates history behind it, and
	/// without it the payload carries only the current attempt — so attempt selection
	/// silently collapses to "latest" and a course configured `attempt_policy = "earliest"`
	/// would be graded on the wrong file, with nothing to show for it.
	///
	/// `attachments` needs no include: it is returned by default.
	pub async fn fetch_submissions(
		&self,
		course_id: u64,
		assignment_id: u64,
	) -> Result<Vec<CanvasSubmissionPayload>, CanvasError> {
		self.paginate(&format!(
			"{}/api/v1/courses/{course_id}/assignments/{assignment_id}/submissions\
			 ?include[]=submission_history&per_page=100",
			self.base_url
		))
		.await
	}

	/// Download one attachment to `dest`, returning the number of bytes written.
	///
	/// The URL is used exactly as Canvas gave it. It carries `download_frd=1` and a
	/// `verifier` token and redirects to the storage host; `reqwest` strips `Authorization`
	/// across that hop, which is correct and harmless because the verifier is what
	/// authorises the final request. The bearer token still serves the first, Canvas-host
	/// hop.
	///
	/// The write is staged through `<dest>.part` so an interrupted download can never be
	/// mistaken for a complete file by the size check that skips re-downloading.
	pub async fn download_attachment(&self, url: &str, dest: &Path) -> Result<u64, CanvasError> {
		let bytes = self.send(url).await?.bytes().await?;

		if let Some(parent) = dest.parent() {
			std::fs::create_dir_all(parent).map_err(|source| CanvasError::IoError {
				path: parent.to_path_buf(),
				source,
			})?;
		}

		let staging = dest.with_file_name(format!(
			"{}.part",
			dest.file_name().unwrap_or_default().to_string_lossy()
		));
		std::fs::write(&staging, &bytes).map_err(|source| CanvasError::IoError {
			path: staging.clone(),
			source,
		})?;
		std::fs::rename(&staging, dest).map_err(|source| CanvasError::IoError {
			path: dest.to_path_buf(),
			source,
		})?;

		Ok(bytes.len() as u64)
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

	/// Save a pulled roster to a CSV file, in the format `load_roster` reads back.
	///
	/// Written with `csv::Writer`, not `writeln!`. The columns are positional and the
	/// fourth one is the Canvas id, so an unescaped comma in a name does not merely mangle
	/// a row — it shifts the student number into the Canvas id column, and the student is
	/// then keyed by their own 学号 read as a Canvas user id, matching no submission and
	/// pushing a grade to a user that does not exist.
	///
	/// The name column is `name`, never `sortable_name`: Canvas builds the latter as
	/// "Last, First", so it contains a comma by construction.
	///
	/// A student with no SIS id gets a blank `student_id` and a populated `canvas_id`;
	/// `load_roster` keys them by Canvas id. Writing the rendered `canvas:12346` form into
	/// `student_id` instead would be rejected on the way back in as a reserved prefix.
	pub fn save_roster_csv(users: &[CanvasUserPayload], path: &Path) -> Result<(), CanvasError> {
		let mut writer = csv::Writer::from_path(path).map_err(|e| CanvasError::IoError {
			path: path.to_path_buf(),
			source: e.into(),
		})?;

		let io = |source: std::io::Error| CanvasError::IoError {
			path: path.to_path_buf(),
			source,
		};

		writer
			.write_record(["name", "class", "student_id", "canvas_id"])
			.map_err(|e| io(e.into()))?;

		let mut sorted: Vec<&CanvasUserPayload> = users.iter().collect();
		sorted.sort_by(|a, b| (&a.sis_user_id, a.id).cmp(&(&b.sis_user_id, b.id)));

		for user in sorted {
			writer
				.write_record([
					user.name.as_deref().unwrap_or(""),
					"",
					user.sis_user_id.as_deref().unwrap_or(""),
					&user.id.to_string(),
				])
				.map_err(|e| io(e.into()))?;
		}

		writer.flush().map_err(io)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn headers(link: &str) -> reqwest::header::HeaderMap {
		let mut map = reqwest::header::HeaderMap::new();
		map.insert(reqwest::header::LINK, link.parse().unwrap());
		map
	}

	#[test]
	fn test_next_link_is_returned_verbatim() {
		// Canvas's cursors are opaque, so the whole URL including its query has to survive.
		let map = headers(
			"<https://c.edu/api/v1/courses/1/users?page=bookmark:WyJhIl0&per_page=100>; rel=\"next\",\
			 <https://c.edu/api/v1/courses/1/users?page=first>; rel=\"first\"",
		);
		assert_eq!(
			next_link(&map).as_deref(),
			Some("https://c.edu/api/v1/courses/1/users?page=bookmark:WyJhIl0&per_page=100")
		);
	}

	#[test]
	fn test_a_last_page_has_no_next_link() {
		let map = headers(
			"<https://c.edu/api/v1/courses/1/users?page=1>; rel=\"current\",\
			 <https://c.edu/api/v1/courses/1/users?page=1>; rel=\"last\"",
		);
		assert_eq!(next_link(&map), None);
		assert_eq!(next_link(&reqwest::header::HeaderMap::new()), None);
	}

	#[test]
	fn test_rel_next_is_not_matched_by_a_substring() {
		// "prev" ends in no `rel="next"`, but a naive `contains` would match `rel="nextish"`.
		let map = headers("<https://c.edu/a>; rel=\"prev\", <https://c.edu/b>; rel=\"nextish\"");
		assert_eq!(next_link(&map), None);
	}

	#[test]
	fn test_roster_csv_escapes_a_comma_in_a_name() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("roster.csv");

		let users = vec![
			CanvasUserPayload {
				id: 12345,
				name: Some("Wu, Alice \"Ali\"".to_string()),
				sortable_name: None,
				sis_user_id: Some("2024010001".to_string()),
				login_id: None,
				email: None,
			},
			CanvasUserPayload {
				id: 12346,
				name: Some("Bob".to_string()),
				sortable_name: None,
				sis_user_id: None,
				login_id: None,
				email: None,
			},
		];

		CanvasClient::save_roster_csv(&users, &path).unwrap();
		let roster = crate::roster::load_roster(&path).unwrap();

		// The comma and the quotes survive, and neither shifts a column.
		let alice = roster
			.lookup_number("2024010001")
			.map(|i| &roster.entries[i])
			.expect("alice keyed by her 学号");
		assert_eq!(alice.name.as_deref(), Some("Wu, Alice \"Ali\""));
		assert_eq!(alice.canvas_user_id, Some(12345));

		// A SIS-less enrollee round-trips under their Canvas id rather than being dropped.
		let bob = roster
			.lookup(&crate::models::StudentKey::CanvasUser(12346))
			.map(|i| &roster.entries[i])
			.expect("bob keyed by his Canvas id");
		assert_eq!(bob.name.as_deref(), Some("Bob"));
		assert_eq!(bob.canvas_user_id, Some(12346));

		assert!(roster.errors().next().is_none());
	}
}
