//! The Canvas fetch layer, driven against a mock HTTP server.
//!
//! These exist because the interesting parts — `Link`-header pagination, the query string
//! Canvas requires for attempt history, redirect handling on a download — are properties of
//! the real `reqwest` client and are invisible to a test that stubs the client out.

use scriptmark::canvas::CanvasClient;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn users_page(ids: &[(u64, &str)]) -> String {
	let rows: Vec<String> = ids
		.iter()
		.map(|(id, sis)| format!(r#"{{"id":{id},"name":"S{id}","sis_user_id":"{sis}"}}"#))
		.collect();
	format!("[{}]", rows.join(","))
}

/// Canvas's pagination cursors are opaque bookmarks in a `Link` header. A numeric `page=`
/// walk — what the client did before — re-reads page one forever on a bookmarked endpoint.
#[tokio::test]
async fn test_pagination_follows_the_link_header_and_stops() {
	let server = MockServer::start().await;

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/users"))
		.and(query_param("page", "bookmark:second"))
		.respond_with(
			ResponseTemplate::new(200)
				.set_body_raw(users_page(&[(3, "2024010003")]), "application/json"),
		)
		.expect(1)
		.mount(&server)
		.await;

	// The first page carries the cursor. No `page` param on the way in.
	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/users"))
		.respond_with(
			ResponseTemplate::new(200)
				.insert_header(
					"Link",
					format!(
						"<{}/api/v1/courses/1/users?page=bookmark:second>; rel=\"next\"",
						server.uri()
					)
					.as_str(),
				)
				.set_body_raw(
					users_page(&[(1, "2024010001"), (2, "2024010002")]),
					"application/json",
				),
		)
		.expect(1)
		.mount(&server)
		.await;

	let client = CanvasClient::with_token(&server.uri(), "t");
	let users = client.pull_roster(1).await.unwrap();

	assert_eq!(users.len(), 3, "both pages, joined");
	assert_eq!(users[2].sis_user_id.as_deref(), Some("2024010003"));
	// `.expect(1)` on each mock is the real assertion that no third request was made.
}

/// Without `include[]=submission_history` Canvas returns only the current attempt, and
/// attempt selection silently collapses to "latest".
#[tokio::test]
async fn test_the_submissions_request_asks_for_history() {
	let server = MockServer::start().await;

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/assignments/2/submissions"))
		.and(query_param("include[]", "submission_history"))
		.and(header("authorization", "Bearer t"))
		.respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
		.expect(1)
		.mount(&server)
		.await;

	let client = CanvasClient::with_token(&server.uri(), "t");
	client.fetch_submissions(1, 2).await.unwrap();
}

/// A page that fails is a hard error, not a short list: these listings establish who is in
/// the course, so a quietly dropped page becomes a cohort of phantom 缺交.
#[tokio::test]
async fn test_a_failing_page_is_an_error_not_a_short_list() {
	let server = MockServer::start().await;

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/users"))
		.respond_with(ResponseTemplate::new(500).set_body_raw("boom", "text/plain"))
		.mount(&server)
		.await;

	let client = CanvasClient::with_token(&server.uri(), "t");
	let error = client.pull_roster(1).await.unwrap_err();
	assert!(
		matches!(
			error,
			scriptmark::canvas::CanvasError::ApiError { status: 500, .. }
		),
		"got {error:?}"
	);
}

/// A server that hands back the URL just fetched must stop the walk, not spin to MAX_PAGES.
#[tokio::test]
async fn test_a_self_referential_next_link_is_refused() {
	let server = MockServer::start().await;
	let here = format!("{}/api/v1/courses/1/users", server.uri());

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/users"))
		.respond_with(
			ResponseTemplate::new(200)
				.insert_header("Link", format!("<{here}>; rel=\"next\"").as_str())
				.set_body_raw(users_page(&[(1, "2024010001")]), "application/json"),
		)
		.mount(&server)
		.await;

	let client = CanvasClient::with_token(&server.uri(), "t");
	let error = client.pull_roster(1).await.unwrap_err();
	assert!(
		matches!(error, scriptmark::canvas::CanvasError::PaginationError(_)),
		"got {error:?}"
	);
}

/// Canvas attachment URLs redirect to a storage host, and `reqwest` drops `Authorization`
/// on the way. That is correct — the `verifier` in the URL is what authorises the final
/// hop — but it means a download must still succeed when the last request carries no token.
#[tokio::test]
async fn test_a_download_survives_a_redirect_that_strips_authorization() {
	let canvas = MockServer::start().await;
	let storage = MockServer::start().await;

	Mock::given(method("GET"))
		.and(path("/files/7/download"))
		.respond_with(ResponseTemplate::new(200).set_body_raw("print(42)", "text/x-python"))
		.expect(1)
		.mount(&storage)
		.await;

	Mock::given(method("GET"))
		.and(path("/files/7"))
		.respond_with(ResponseTemplate::new(302).insert_header(
			"Location",
			format!("{}/files/7/download", storage.uri()).as_str(),
		))
		.expect(1)
		.mount(&canvas)
		.await;

	let dir = tempfile::tempdir().unwrap();
	let dest = dir.path().join("7").join("lab1.py");

	let client = CanvasClient::with_token(&canvas.uri(), "t");
	let written = client
		.download_attachment(&format!("{}/files/7?verifier=abc", canvas.uri()), &dest)
		.await
		.unwrap();

	assert_eq!(written, 9);
	assert_eq!(std::fs::read_to_string(&dest).unwrap(), "print(42)");
	// The staging file must not survive a successful download.
	assert!(!dest.with_file_name("lab1.py.part").exists());
}

/// A failed download leaves nothing behind that a later size check could mistake for a
/// complete file.
#[tokio::test]
async fn test_a_failed_download_writes_no_partial_file() {
	let server = MockServer::start().await;

	Mock::given(method("GET"))
		.and(path("/files/9"))
		.respond_with(ResponseTemplate::new(404).set_body_raw("gone", "text/plain"))
		.mount(&server)
		.await;

	let dir = tempfile::tempdir().unwrap();
	let dest = dir.path().join("9").join("lab1.py");

	let client = CanvasClient::with_token(&server.uri(), "t");
	let error = client
		.download_attachment(&format!("{}/files/9", server.uri()), &dest)
		.await
		.unwrap_err();

	assert!(
		matches!(
			error,
			scriptmark::canvas::CanvasError::ApiError { status: 404, .. }
		),
		"got {error:?}"
	);
	assert!(!dest.exists());
	assert!(!dest.with_file_name("lab1.py.part").exists());
}
