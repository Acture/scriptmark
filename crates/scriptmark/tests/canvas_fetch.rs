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

/// The whole bundle round-trip: fetch writes it, a later offline run reads it back and
/// normalises to the same thing.
///
/// The fixture carries a **zip** attachment on purpose. `attachments.json` records only
/// delivery, so if the loader did not re-expand archives from disk, this student would come
/// back with the `.zip` as their only candidate file, `detect_language` would refuse it, and
/// they would be reported `SubmittedEmpty` — while the bundle the fetch wrote said
/// `Executable`. Asserting the in-archive `entry` survives is what additionally rejects a
/// directory-scan implementation, which cannot recover it: expansion flattens
/// `src/Lab5.py` to `Lab5.py`.
#[tokio::test]
async fn test_a_bundle_round_trips_a_zip_attachment_and_a_failed_download() {
	use scriptmark::canvas::bundle;
	use scriptmark::models::{
		Assignment, AttemptPolicy, DiagnosticKind, FileOrigin, SubmissionOutcome,
	};
	use std::sync::Arc;

	let server = MockServer::start().await;

	// A zip holding one nested python file.
	let mut zip_bytes = Vec::new();
	{
		use std::io::Write as _;
		let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
		zip.start_file("src/Lab5.py", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(b"def foo(): return 42").unwrap();
		zip.finish().unwrap();
	}

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/users"))
		.respond_with(ResponseTemplate::new(200).set_body_raw(
			r#"[{"id":11,"name":"Alice","sis_user_id":"2024010001"},
			    {"id":12,"name":"Bob","sis_user_id":"2024010002"}]"#,
			"application/json",
		))
		.mount(&server)
		.await;

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/assignments/2"))
		.respond_with(
			ResponseTemplate::new(200)
				.set_body_raw(r#"{"id":2,"name":"Lab 5"}"#, "application/json"),
		)
		.mount(&server)
		.await;

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/assignments/2/submissions"))
		.respond_with(ResponseTemplate::new(200).set_body_raw(
			format!(
				r#"[{{"id":100,"user_id":11,"attempt":1,"workflow_state":"submitted",
				      "submission_type":"online_upload","submitted_at":"2026-03-01T00:00:00Z",
				      "attachments":[{{"id":501,"display_name":"work.zip","content-type":"application/zip","url":"{uri}/files/501"}}]}},
				    {{"id":101,"user_id":12,"attempt":1,"workflow_state":"submitted",
				      "submission_type":"online_upload","submitted_at":"2026-03-01T00:00:00Z",
				      "attachments":[{{"id":502,"display_name":"lab5.py","content-type":"text/x-python","url":"{uri}/files/502"}}]}}]"#,
				uri = server.uri()
			),
			"application/json",
		))
		.mount(&server)
		.await;

	Mock::given(method("GET"))
		.and(path("/files/501"))
		.respond_with(ResponseTemplate::new(200).set_body_raw(zip_bytes, "application/zip"))
		.mount(&server)
		.await;

	// Bob's file is refused. He must keep his identity and not become 缺交.
	Mock::given(method("GET"))
		.and(path("/files/502"))
		.respond_with(ResponseTemplate::new(502).set_body_raw("bad gateway", "text/plain"))
		.mount(&server)
		.await;

	let dir = tempfile::tempdir().unwrap();
	let root = dir.path().join("hw5");

	let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
	bundle::fetch(client, 1, 2, &root, 1, |_| {}).await.unwrap();

	// Everything below is offline — this is what `grade --canvas` does.
	let (payload, downloads, extra) = bundle::load(&root).unwrap();
	assert_eq!(payload.assignment_name.as_deref(), Some("Lab 5"));

	let input = scriptmark::input::canvas::normalize(
		&payload,
		None,
		&downloads,
		AttemptPolicy::Latest,
		Assignment::default(),
	);
	let input = bundle::merge_diagnostics(input, extra);

	let alice = input
		.students
		.iter()
		.find(|s| s.key().raw() == "2024010001")
		.expect("alice");
	assert_eq!(alice.outcome(), SubmissionOutcome::Executable);
	assert_eq!(alice.files().len(), 1, "the zip's contents, not the zip");
	assert_eq!(
		alice.files()[0].origin,
		FileOrigin::Attachment {
			attempt: 1,
			attachment_id: 501,
			entry: Some("src/Lab5.py".to_string()),
		},
		"the in-archive path must survive the reload"
	);

	let bob = input
		.students
		.iter()
		.find(|s| s.key().raw() == "2024010002")
		.expect("bob");
	// Attempted and refused is not 缺交.
	assert_ne!(bob.outcome(), SubmissionOutcome::NotSubmitted);
	assert!(
		input.diagnostics.iter().any(|d| matches!(
			&d.kind,
			DiagnosticKind::AttachmentUnavailable { key, attachment_id, .. }
				if key == "2024010002" && *attachment_id == 502
		)),
		"the failure must name Bob, got {:?}",
		input.diagnostics
	);
}

/// Canvas repeats a carried-forward attachment in every later attempt. Walking attempts
/// would fetch the same bytes once per attempt and report a file count no teacher would
/// recognise.
#[tokio::test]
async fn test_an_attachment_carried_across_attempts_is_fetched_once() {
	use scriptmark::canvas::bundle;
	use std::sync::Arc;

	let server = MockServer::start().await;
	mount_minimal_course(&server).await;

	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/assignments/2/submissions"))
		.respond_with(ResponseTemplate::new(200).set_body_raw(
			format!(
				r#"[{{"id":100,"user_id":11,"attempt":2,"workflow_state":"submitted",
				      "submission_type":"online_upload",
				      "attachments":[{{"id":700,"display_name":"a.py","url":"{uri}/files/700"}}],
				      "submission_history":[
				        {{"id":100,"user_id":11,"attempt":1,"attachments":[{{"id":700,"display_name":"a.py","url":"{uri}/files/700"}}]}},
				        {{"id":100,"user_id":11,"attempt":2,"attachments":[{{"id":700,"display_name":"a.py","url":"{uri}/files/700"}}]}}
				      ]}}]"#,
				uri = server.uri()
			),
			"application/json",
		))
		.mount(&server)
		.await;

	// `.expect(1)` is the assertion: three mentions, one GET.
	Mock::given(method("GET"))
		.and(path("/files/700"))
		.respond_with(ResponseTemplate::new(200).set_body_raw("print(1)", "text/x-python"))
		.expect(1)
		.mount(&server)
		.await;

	let dir = tempfile::tempdir().unwrap();
	let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
	bundle::fetch(client, 1, 2, dir.path(), 1, |_| {})
		.await
		.unwrap();
}

/// Re-fetch must skip what it already has — but only when the size agrees.
///
/// The skip direction alone is not worth testing: `if path.exists() { continue }` satisfies
/// it, and that is precisely the bug. The two cases below discriminate.
#[tokio::test]
async fn test_refetch_skips_a_matching_file_but_replaces_a_truncated_one() {
	use scriptmark::canvas::bundle;
	use std::sync::Arc;

	let body = "print(42)"; // 9 bytes
	let dir = tempfile::tempdir().unwrap();
	let root = dir.path().join("hw");

	// --- (a) size matches: zero GETs on the second fetch ---
	{
		let server = MockServer::start().await;
		mount_minimal_course(&server).await;
		mount_one_upload(&server, 800, "a.py", Some(body.len() as u64)).await;
		Mock::given(method("GET"))
			.and(path("/files/800"))
			.respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/x-python"))
			.expect(1)
			.mount(&server)
			.await;

		let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
		bundle::fetch(Arc::clone(&client), 1, 2, &root, 1, |_| {})
			.await
			.unwrap();
		// Second pass over the same bundle: the mock still expects exactly one GET.
		bundle::fetch(client, 1, 2, &root, 1, |_| {}).await.unwrap();
	}

	let stored = root.join("attachments/800/a.py");
	assert_eq!(std::fs::read_to_string(&stored).unwrap(), body);

	// --- (b) the file on disk is short: it must be replaced, not trusted ---
	std::fs::write(&stored, "pri").unwrap();
	{
		let server = MockServer::start().await;
		mount_minimal_course(&server).await;
		mount_one_upload(&server, 800, "a.py", Some(body.len() as u64)).await;
		Mock::given(method("GET"))
			.and(path("/files/800"))
			.respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/x-python"))
			.expect(1)
			.mount(&server)
			.await;

		let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
		bundle::fetch(client, 1, 2, &root, 1, |_| {}).await.unwrap();
	}
	assert_eq!(
		std::fs::read_to_string(&stored).unwrap(),
		body,
		"a truncated file must be re-downloaded, not mistaken for complete"
	);

	// --- (c) Canvas reports no size: never assume what is there is whole ---
	std::fs::write(&stored, "pri").unwrap();
	{
		let server = MockServer::start().await;
		mount_minimal_course(&server).await;
		mount_one_upload(&server, 800, "a.py", None).await;
		Mock::given(method("GET"))
			.and(path("/files/800"))
			.respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/x-python"))
			.expect(1)
			.mount(&server)
			.await;

		let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
		bundle::fetch(client, 1, 2, &root, 1, |_| {}).await.unwrap();
	}
	assert_eq!(std::fs::read_to_string(&stored).unwrap(), body);
}

async fn mount_minimal_course(server: &MockServer) {
	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/users"))
		.respond_with(ResponseTemplate::new(200).set_body_raw(
			r#"[{"id":11,"name":"Alice","sis_user_id":"2024010001"}]"#,
			"application/json",
		))
		.mount(server)
		.await;
	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/assignments/2"))
		.respond_with(
			ResponseTemplate::new(200).set_body_raw(r#"{"id":2,"name":"Lab"}"#, "application/json"),
		)
		.mount(server)
		.await;
}

async fn mount_one_upload(server: &MockServer, id: u64, name: &str, size: Option<u64>) {
	let size_field = size.map(|s| format!(r#","size":{s}"#)).unwrap_or_default();
	Mock::given(method("GET"))
		.and(path("/api/v1/courses/1/assignments/2/submissions"))
		.respond_with(ResponseTemplate::new(200).set_body_raw(
			format!(
				r#"[{{"id":100,"user_id":11,"attempt":1,"workflow_state":"submitted",
				      "submission_type":"online_upload",
				      "attachments":[{{"id":{id},"display_name":"{name}","url":"{uri}/files/{id}"{size_field}}}]}}]"#,
				uri = server.uri()
			),
			"application/json",
		))
		.mount(server)
		.await;
}

/// A listing that fails is fatal — but it must not take the previous bundle with it.
///
/// The two JSON files are written last, so a run that dies during fetching leaves the last
/// good manifest and the files it described exactly where they were. Without that, a
/// transient 500 would cost a teacher the download they already had.
#[tokio::test]
async fn test_a_failed_refetch_leaves_the_previous_bundle_intact() {
	use scriptmark::canvas::bundle;
	use std::sync::Arc;

	let dir = tempfile::tempdir().unwrap();
	let root = dir.path().join("hw");

	// A good fetch first.
	{
		let server = MockServer::start().await;
		mount_minimal_course(&server).await;
		mount_one_upload(&server, 900, "a.py", Some(8)).await;
		Mock::given(method("GET"))
			.and(path("/files/900"))
			.respond_with(ResponseTemplate::new(200).set_body_raw("print(1)", "text/x-python"))
			.mount(&server)
			.await;

		let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
		bundle::fetch(client, 1, 2, &root, 1, |_| {}).await.unwrap();
	}

	let (payload, downloads, _) = bundle::load(&root).unwrap();
	assert_eq!(payload.submissions.len(), 1);
	assert!(downloads.get(&900).unwrap().is_ok());

	// Now the submissions listing fails.
	{
		let server = MockServer::start().await;
		mount_minimal_course(&server).await;
		Mock::given(method("GET"))
			.and(path("/api/v1/courses/1/assignments/2/submissions"))
			.respond_with(ResponseTemplate::new(500).set_body_raw("boom", "text/plain"))
			.mount(&server)
			.await;

		let client = Arc::new(CanvasClient::with_token(&server.uri(), "t"));
		bundle::fetch(client, 1, 2, &root, 1, |_| {})
			.await
			.expect_err("a failed listing must be fatal, not a short list");
	}

	// The bundle still reads, and still holds what the good fetch downloaded.
	let (payload, downloads, _) = bundle::load(&root).unwrap();
	assert_eq!(payload.submissions.len(), 1);
	let stored = downloads.get(&900).unwrap().as_ref().unwrap();
	assert_eq!(std::fs::read_to_string(&stored.path).unwrap(), "print(1)");
}
