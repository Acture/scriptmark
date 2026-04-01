use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, Tab};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // tabs
            Constraint::Min(10),   // content
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);
    draw_tabs(f, chunks[1], app);

    match app.tab {
        Tab::Students => draw_students(f, chunks[2], app),
        Tab::Sessions => draw_sessions(f, chunks[2], app),
        Tab::Similarity => draw_similarity(f, chunks[2], app),
    }

    draw_footer(f, chunks[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let session_info = app
        .current_session
        .and_then(|i| app.sessions.get(i))
        .map(|s| {
            format!(
                "  |  {}  |  {} students  |  avg {:.1}",
                s.assignment, s.student_count, s.avg_grade
            )
        })
        .unwrap_or_default();

    let title = Line::from(vec![
        Span::styled(
            " ScriptMark",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(session_info, Style::default().fg(Color::DarkGray)),
    ]);

    f.render_widget(
        Paragraph::new(title).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let tabs = Tabs::new(vec!["Students", "Sessions", "Similarity"])
        .select(match app.tab {
            Tab::Students => 0,
            Tab::Sessions => 1,
            Tab::Similarity => 2,
        })
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(Color::DarkGray));

    f.render_widget(tabs, area);
}

fn draw_students(f: &mut Frame, area: Rect, app: &App) {
    if app.detail_open {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);
        draw_student_list(f, chunks[0], app);
        draw_detail(f, chunks[1], app);
    } else {
        draw_student_list(f, area, app);
    }
}

fn draw_student_list(f: &mut Frame, area: Rect, app: &App) {
    let filtered = app.filtered_results();

    let header = Row::new(vec!["Name", "ID", "Grade", "Rate", "Pass/Total"])
        .style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let rows: Vec<Row> = filtered
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let grade_color = if r.final_grade >= 90.0 {
                Color::Green
            } else if r.final_grade >= 70.0 {
                Color::Blue
            } else if r.final_grade >= 60.0 {
                Color::Yellow
            } else {
                Color::Red
            };

            let style = if i == app.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(r.student_name.as_deref().unwrap_or("N/A")),
                Cell::from(r.student_id.as_str()),
                Cell::from(format!("{:.1}", r.final_grade)).style(Style::default().fg(grade_color)),
                Cell::from(format!("{:.0}%", r.pass_rate)),
                Cell::from(format!("{}/{}", r.passed_cases, r.total_cases)),
            ])
            .style(style)
        })
        .collect();

    let search_title = if app.searching {
        format!(" Students [/{}] ", app.search)
    } else if !app.search.is_empty() {
        format!(" Students [filter: {}] ", app.search)
    } else {
        " Students ".to_string()
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(15),
            Constraint::Min(12),
            Constraint::Min(7),
            Constraint::Min(6),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(search_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(table, area);
}

fn draw_detail(f: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(json) = &app.detail_json
        && let Ok(report) = serde_json::from_str::<scriptmark_core::models::StudentReport>(json)
    {
        for tr in &report.test_results {
            lines.push(Line::from(Span::styled(
                format!("--- {} ---", tr.spec_name),
                Style::default().fg(Color::Cyan),
            )));
            for case in &tr.cases {
                let (icon, color) = match case.status {
                    scriptmark_core::models::TestStatus::Passed => ("v", Color::Green),
                    scriptmark_core::models::TestStatus::Failed => ("x", Color::Red),
                    scriptmark_core::models::TestStatus::Timeout => ("T", Color::Yellow),
                    _ => ("!", Color::Red),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), Style::default().fg(color)),
                    Span::raw(case.case_name.clone()),
                ]));
                if let Some(failure) = &case.failure {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", failure.message),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }
    }

    if lines.is_empty() {
        let text = app.detail_json.as_deref().unwrap_or("No details");
        lines.push(Line::from(text.to_owned()));
    }

    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Details ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: true })
        .scroll((0, 0));

    f.render_widget(detail, area);
}

fn draw_sessions(f: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec!["ID", "Assignment", "Students", "Avg Grade", "Date"])
        .style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == app.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(s.id.to_string()).style(Style::default().fg(Color::Cyan)),
                Cell::from(s.assignment.as_str()),
                Cell::from(s.student_count.to_string()),
                Cell::from(format!("{:.1}", s.avg_grade)),
                Cell::from(s.created_at.as_str()).style(Style::default().fg(Color::DarkGray)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(4),
            Constraint::Min(20),
            Constraint::Min(10),
            Constraint::Min(10),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(" Sessions ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(table, area);
}

fn draw_similarity(f: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec![
        "Score",
        "Student A",
        "Student B",
        "Style",
        "Structure",
    ])
    .style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .similarity
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let score_color = if p.score > 0.9 {
                Color::Red
            } else if p.score > 0.75 {
                Color::Yellow
            } else {
                Color::Green
            };

            let style = if i == app.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(format!("{:.1}%", p.score * 100.0)).style(
                    Style::default()
                        .fg(score_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(p.student_a.as_str()),
                Cell::from(p.student_b.as_str()),
                Cell::from(format!("{:.0}%", p.style_score * 100.0)),
                Cell::from(format!("{:.0}%", p.structure_score * 100.0)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(8),
            Constraint::Min(15),
            Constraint::Min(15),
            Constraint::Min(8),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(" Similarity ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(table, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let help = if app.searching {
        " Type to search | Enter confirm | Esc cancel "
    } else {
        " jk navigate | Enter details | Tab switch | / search | q quit "
    };

    let footer = Paragraph::new(Span::styled(help, Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, area);
}
