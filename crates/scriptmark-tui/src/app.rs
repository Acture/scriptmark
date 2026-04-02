use std::io;
use std::path::Path;

use anyhow::Result;
use crossterm::{
	event::{self, Event, KeyCode, KeyModifiers},
	execute,
	terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::prelude::*;
use scriptmark_db::{Database, ResultRow, Session};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
	Students,
	Sessions,
	Similarity,
}

pub struct App {
	pub db: Database,
	pub sessions: Vec<Session>,
	pub current_session: Option<usize>,
	pub results: Vec<ResultRow>,
	pub tab: Tab,
	pub selected: usize,
	pub detail_open: bool,
	pub detail_json: Option<String>,
	pub search: String,
	pub searching: bool,
	pub similarity: Vec<scriptmark_core::similarity::SimilarityPair>,
	pub should_quit: bool,
}

impl App {
	pub fn new(db_path: &Path) -> Result<Self> {
		let db = Database::open(db_path)?;
		let sessions = db.list_sessions()?;
		let (results, similarity) = if let Some(s) = sessions.first() {
			let r = db.get_results(s.id)?;
			let sim = db.get_similarity(s.id)?;
			(r, sim)
		} else {
			(vec![], vec![])
		};

		Ok(Self {
			db,
			current_session: if sessions.is_empty() { None } else { Some(0) },
			sessions,
			results,
			tab: Tab::Students,
			selected: 0,
			detail_open: false,
			detail_json: None,
			search: String::new(),
			searching: false,
			similarity,
			should_quit: false,
		})
	}

	pub fn filtered_results(&self) -> Vec<&ResultRow> {
		if self.search.is_empty() {
			self.results.iter().collect()
		} else {
			let s = self.search.to_lowercase();
			self.results
				.iter()
				.filter(|r| {
					r.student_id.to_lowercase().contains(&s)
						|| r.student_name
							.as_deref()
							.unwrap_or("")
							.to_lowercase()
							.contains(&s)
				})
				.collect()
		}
	}

	fn load_session(&mut self, idx: usize) {
		if idx < self.sessions.len() {
			self.current_session = Some(idx);
			let sid = self.sessions[idx].id;
			self.results = self.db.get_results(sid).unwrap_or_default();
			self.similarity = self.db.get_similarity(sid).unwrap_or_default();
			self.selected = 0;
			self.detail_open = false;
		}
	}

	fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
		if self.searching {
			match key {
				KeyCode::Esc => {
					self.searching = false;
					self.search.clear();
				}
				KeyCode::Enter => {
					self.searching = false;
				}
				KeyCode::Backspace => {
					self.search.pop();
				}
				KeyCode::Char(c) => {
					self.search.push(c);
				}
				_ => {}
			}
			return;
		}

		match key {
			KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
			KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
				self.should_quit = true;
			}
			KeyCode::Tab => {
				self.tab = match self.tab {
					Tab::Students => Tab::Sessions,
					Tab::Sessions => Tab::Similarity,
					Tab::Similarity => Tab::Students,
				};
				self.selected = 0;
				self.detail_open = false;
			}
			KeyCode::Down | KeyCode::Char('j') => {
				let max = match self.tab {
					Tab::Students => self.filtered_results().len(),
					Tab::Sessions => self.sessions.len(),
					Tab::Similarity => self.similarity.len(),
				};
				if self.selected + 1 < max {
					self.selected += 1;
				}
			}
			KeyCode::Up | KeyCode::Char('k') => {
				if self.selected > 0 {
					self.selected -= 1;
				}
			}
			KeyCode::Enter => match self.tab {
				Tab::Students => {
					let filtered = self.filtered_results();
					if let Some(row) = filtered.get(self.selected)
						&& let Some(sid) = self.current_session
						&& let Ok(Some(report)) = self
							.db
							.get_student_details(self.sessions[sid].id, &row.student_id)
					{
						self.detail_json = serde_json::to_string_pretty(&report).ok();
						self.detail_open = !self.detail_open;
					}
				}
				Tab::Sessions => {
					self.load_session(self.selected);
					self.tab = Tab::Students;
				}
				Tab::Similarity => {}
			},
			KeyCode::Char('/') => {
				self.searching = true;
				self.search.clear();
			}
			_ => {}
		}
	}
}

pub fn run_tui(db_path: &Path) -> Result<()> {
	enable_raw_mode()?;
	let mut stdout = io::stdout();
	execute!(stdout, EnterAlternateScreen)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = Terminal::new(backend)?;

	let mut app = App::new(db_path)?;

	loop {
		terminal.draw(|f| crate::ui::draw(f, &app))?;

		if event::poll(std::time::Duration::from_millis(100))?
			&& let Event::Key(key) = event::read()?
		{
			app.handle_key(key.code, key.modifiers);
		}

		if app.should_quit {
			break;
		}
	}

	disable_raw_mode()?;
	execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
	Ok(())
}
