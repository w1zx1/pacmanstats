use crate::logparse::{Event, Kind};
use crate::model::{Day, DiffKind};
use crossterm::event::{self, Event as CEvent, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs},
    Frame,
};
use std::io;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    All,
    Installed,
    Upgraded,
    Removed,
    Reinstalled,
    Diff,
}

impl View {
    const ALL: [View; 6] = [
        View::All,
        View::Installed,
        View::Upgraded,
        View::Removed,
        View::Reinstalled,
        View::Diff,
    ];

    fn title(self) -> &'static str {
        match self {
            View::All => "All (1)",
            View::Installed => "Installed (2)",
            View::Upgraded => "Upgraded (3)",
            View::Removed => "Removed (4)",
            View::Reinstalled => "Reinst (5)",
            View::Diff => "Diff (6)",
        }
    }

    fn matches(self, kind: crate::logparse::Kind) -> bool {
        match self {
            View::All => true,
            View::Installed => kind == crate::logparse::Kind::Installed,
            View::Upgraded => {
                kind == crate::logparse::Kind::Upgraded
                    || kind == crate::logparse::Kind::Downgraded
            }
            View::Removed => kind == crate::logparse::Kind::Removed,
            View::Reinstalled => kind == crate::logparse::Kind::Reinstalled,
            View::Diff => true, // not used for events
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Days,
    Pkgs,
}

struct Ui {
    days: Vec<Day>, // newest first
    day_idx: usize,
    /// Scroll offset of the packages pane (first visible row).
    pkg_scroll: usize,
    /// Last known viewport height of the packages pane, for paging.
    pkg_h: usize,
    view: View,
    focus: Focus,
    query: String,
    editing_filter: bool,
    total_events: usize,
}

impl Ui {
    fn selected_day(&self) -> Option<&Day> {
        self.days.get(self.day_idx)
    }

    fn filtered_events(&self) -> Vec<&Event> {
        let Some(day) = self.selected_day() else {
            return vec![];
        };
        let q = self.query.to_lowercase();
        day.events
            .iter()
            .filter(|e| self.view.matches(e.kind))
            .filter(|e| q.is_empty() || e.pkg.to_lowercase().contains(&q))
            .collect()
    }

    fn filtered_diff(&self) -> Vec<(DiffKind, &str, Option<&str>, Option<&str>)> {
        let Some(day) = self.selected_day() else {
            return vec![];
        };
        let q = self.query.to_lowercase();
        day.diff
            .iter()
            .filter(|d| q.is_empty() || d.pkg.to_lowercase().contains(&q))
            .map(|d| {
                (
                    d.kind(),
                    d.pkg.as_str(),
                    d.before.as_deref(),
                    d.after.as_deref(),
                )
            })
            .collect()
    }

    /// Max scroll offset so the last row stays reachable without
    /// leaving the pane blank.
    fn pkg_max_scroll(&self) -> usize {
        self.pkg_rows_len().saturating_sub(self.pkg_h.max(1))
    }

    fn pkg_rows_len(&self) -> usize {
        if self.view == View::Diff {
            self.filtered_diff().len()
        } else {
            self.filtered_events().len()
        }
    }

    fn clamp(&mut self) {
        if self.days.is_empty() {
            self.day_idx = 0;
        } else if self.day_idx >= self.days.len() {
            self.day_idx = self.days.len() - 1;
        }
        let max = self.pkg_max_scroll();
        if self.pkg_scroll > max {
            self.pkg_scroll = max;
        }
    }
}

fn kind_style(kind: Kind) -> Style {
    match kind {
        Kind::Installed => Style::default().fg(Color::Green),
        Kind::Upgraded => Style::default().fg(Color::Yellow),
        Kind::Downgraded => Style::default().fg(Color::Magenta),
        Kind::Removed => Style::default().fg(Color::Red),
        Kind::Reinstalled => Style::default().fg(Color::Cyan),
    }
}

fn render(f: &mut Frame, ui: &mut Ui, log_path: &Path, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);

    // Header
    let total_days = ui.days.len();
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "pacmanstats ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "— {}  •  {} days, {} events",
                log_path.display(),
                total_days,
                ui.total_events
            )),
        ]),
        Line::from(vec![Span::styled(
            match ui.focus {
                Focus::Days => "focus: days  •  Tab switch pane",
                Focus::Pkgs => "focus: packages  •  Tab switch pane",
            },
            Style::default().fg(Color::DarkGray),
        )]),
    ]);
    f.render_widget(header, chunks[0]);

    // Tabs
    let titles: Vec<Line> = View::ALL.iter().map(|v| Line::from(v.title())).collect();
    let tab_idx = View::ALL.iter().position(|v| *v == ui.view).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" view "))
        .select(tab_idx)
        .style(Style::default())
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[1]);

    // Main split
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(chunks[2]);

    // Days list
    let day_items: Vec<ListItem> = ui
        .days
        .iter()
        .map(|d| {
            let line = Line::from(vec![
                Span::raw(format!(
                    "{}  +{} ~{} -{}",
                    d.date, d.n_installed, d.n_upgraded, d.n_removed
                )),
                if d.n_reinstalled > 0 {
                    Span::styled(
                        format!(" ={}", d.n_reinstalled),
                        Style::default().fg(Color::Cyan),
                    )
                } else {
                    Span::raw("")
                },
                Span::styled(
                    format!("  Δ{}", d.diff.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();
    let days_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" days ({}) ", ui.days.len()))
        .border_style(if ui.focus == Focus::Days {
            Style::default().fg(Color::White)
        } else {
            Style::default()
        });
    let mut day_state = ListState::default();
    day_state.select(if ui.days.is_empty() {
        None
    } else {
        Some(ui.day_idx)
    });
    let days_list = List::new(day_items)
        .block(days_block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(days_list, main[0], &mut day_state);

    // Packages / diff pane
    let (title, items) = if ui.view == View::Diff {
        let rows = ui.filtered_diff();
        let title = format!(
            " diff {} (net change, churn cancelled) ",
            ui.selected_day()
                .map(|d| d.date.to_string())
                .unwrap_or_default()
        );
        let items: Vec<ListItem> = rows
            .iter()
            .map(|(k, pkg, before, after)| {
                let (sym, style, detail) = match k {
                    DiffKind::Added => (
                        '+',
                        Style::default().fg(Color::Green),
                        format!("{} {}", pkg, after.unwrap_or("?")),
                    ),
                    DiffKind::Removed => (
                        '-',
                        Style::default().fg(Color::Red),
                        format!("{} {}", pkg, before.unwrap_or("?")),
                    ),
                    DiffKind::Changed => (
                        '~',
                        Style::default().fg(Color::Yellow),
                        format!(
                            "{} {} -> {}",
                            pkg,
                            before.unwrap_or("∅"),
                            after.unwrap_or("∅")
                        ),
                    ),
                };
                let line = Line::from(vec![
                    Span::styled(format!("{} ", sym), style),
                    Span::raw(detail),
                ]);
                ListItem::new(line)
            })
            .collect();
        (title, items)
    } else {
        let rows = ui.filtered_events();
        let title = format!(
            " {} {} ({} shown) ",
            ui.selected_day()
                .map(|d| d.date.to_string())
                .unwrap_or_default(),
            ui.view.title(),
            rows.len()
        );
        let items: Vec<ListItem> = rows
            .iter()
            .map(|e| {
                let st = kind_style(e.kind);
                let line = Line::from(vec![
                    Span::styled(format!("{} ", e.time), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{} ", e.kind.short()), st),
                    Span::raw(e.pkg.clone()),
                    Span::raw(" "),
                    Span::styled(e.version_display(), st),
                ]);
                ListItem::new(line)
            })
            .collect();
        (title, items)
    };

    let pkgs_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if ui.focus == Focus::Pkgs {
            Style::default().fg(Color::White)
        } else {
            Style::default()
        });
    if items.is_empty() {
        let empty = Paragraph::new(if ui.days.is_empty() {
            "no ALPM events in log"
        } else if !ui.query.is_empty() {
            "no matches for filter — Esc to clear"
        } else if ui.view == View::Diff {
            "net zero — everything installed that day was also removed (or vice versa)"
        } else {
            "no events in this view for the day"
        })
        .block(pkgs_block)
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, main[1]);
    } else {
        // Plain viewport scroll: no per-row cursor, the list itself moves.
        let height = main[1].height.saturating_sub(2) as usize;
        let height = height.max(1);
        ui.pkg_h = height;
        let start = ui.pkg_scroll.min(items.len().saturating_sub(height));
        let visible: Vec<ListItem> = items.into_iter().skip(start).take(height).collect();
        let list = List::new(visible).block(pkgs_block);
        f.render_widget(list, main[1]);
    }

    // Footer: hint centered; while typing a filter the whole hint is
    // replaced by the search input.
    if ui.editing_filter {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled("filter: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{}▌", ui.query)),
        ]))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(footer, chunks[3]);
    } else {
        let mut spans = vec![
            Span::raw("q quit  •  Tab focus  •  ↑↓ move  •  1-6 view  •  "),
            Span::styled("/ filter", Style::default().fg(Color::Yellow)),
            Span::raw("  •  Esc clear"),
        ];
        if !ui.query.is_empty() {
            spans.push(Span::raw("  •  "));
            spans.push(Span::styled(
                format!("filter: `{}`", ui.query),
                Style::default().fg(Color::Yellow),
            ));
        }
        let footer = Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(footer, chunks[3]);
    }
}

pub fn run(days_asc: Vec<Day>, log_path: &Path) -> io::Result<()> {
    let mut days = days_asc;
    days.reverse(); // newest first
    let total_events: usize = days.iter().map(|d| d.events.len()).sum();

    let mut terminal = ratatui::init();
    let mut ui = Ui {
        days,
        day_idx: 0,
        pkg_scroll: 0,
        pkg_h: 0,
        view: View::All,
        focus: Focus::Days,
        query: String::new(),
        editing_filter: false,
        total_events,
    };

    loop {
        ui.clamp();
        terminal.draw(|f| render(f, &mut ui, log_path, f.area()))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let CEvent::Key(key) = event::read()? else {
            continue;
        };

        // Filter editing mode captures everything except Esc/Enter.
        if ui.editing_filter {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => ui.editing_filter = false,
                KeyCode::Backspace => {
                    ui.query.pop();
                    ui.pkg_scroll = 0;
                }
                KeyCode::Char(c) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        ui.query.push(c);
                        ui.pkg_scroll = 0;
                    }
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => break,
            KeyCode::Esc => {
                if !ui.query.is_empty() {
                    ui.query.clear();
                    ui.pkg_scroll = 0;
                }
            }
            KeyCode::Char('/') => {
                ui.editing_filter = true;
            }
            KeyCode::Tab => {
                ui.focus = match ui.focus {
                    Focus::Days => Focus::Pkgs,
                    Focus::Pkgs => Focus::Days,
                };
            }
            KeyCode::Left | KeyCode::Char('h') => ui.focus = Focus::Days,
            KeyCode::Right | KeyCode::Char('l') => ui.focus = Focus::Pkgs,
            KeyCode::Char('1') => {
                ui.view = View::All;
                ui.pkg_scroll = 0;
            }
            KeyCode::Char('2') => {
                ui.view = View::Installed;
                ui.pkg_scroll = 0;
            }
            KeyCode::Char('3') => {
                ui.view = View::Upgraded;
                ui.pkg_scroll = 0;
            }
            KeyCode::Char('4') => {
                ui.view = View::Removed;
                ui.pkg_scroll = 0;
            }
            KeyCode::Char('5') => {
                ui.view = View::Reinstalled;
                ui.pkg_scroll = 0;
            }
            KeyCode::Char('6') | KeyCode::Char('d') => {
                ui.view = View::Diff;
                ui.pkg_scroll = 0;
            }
            KeyCode::Up => match ui.focus {
                Focus::Days => {
                    ui.day_idx = ui.day_idx.saturating_sub(1);
                    ui.pkg_scroll = 0;
                }
                Focus::Pkgs => ui.pkg_scroll = ui.pkg_scroll.saturating_sub(1),
            },
            KeyCode::Down => match ui.focus {
                Focus::Days => {
                    if ui.day_idx + 1 < ui.days.len() {
                        ui.day_idx += 1;
                        ui.pkg_scroll = 0;
                    }
                }
                Focus::Pkgs => {
                    ui.pkg_scroll = (ui.pkg_scroll + 1).min(ui.pkg_max_scroll());
                }
            },
            KeyCode::PageUp => match ui.focus {
                Focus::Days => {
                    ui.day_idx = ui.day_idx.saturating_sub(10);
                    ui.pkg_scroll = 0;
                }
                Focus::Pkgs => {
                    let page = ui.pkg_h.max(1);
                    ui.pkg_scroll = ui.pkg_scroll.saturating_sub(page);
                }
            },
            KeyCode::PageDown => match ui.focus {
                Focus::Days => {
                    ui.day_idx = (ui.day_idx + 10).min(ui.days.len().saturating_sub(1));
                    ui.pkg_scroll = 0;
                }
                Focus::Pkgs => {
                    let page = ui.pkg_h.max(1);
                    ui.pkg_scroll = (ui.pkg_scroll + page).min(ui.pkg_max_scroll());
                }
            },
            KeyCode::Home => match ui.focus {
                Focus::Days => {
                    ui.day_idx = 0;
                    ui.pkg_scroll = 0;
                }
                Focus::Pkgs => ui.pkg_scroll = 0,
            },
            KeyCode::End => match ui.focus {
                Focus::Days => {
                    ui.day_idx = ui.days.len().saturating_sub(1);
                    ui.pkg_scroll = 0;
                }
                Focus::Pkgs => {
                    ui.pkg_scroll = ui.pkg_max_scroll();
                }
            },
            _ => {}
        }
    }

    ratatui::restore();
    Ok(())
}
