use ratatui::prelude::*;
use ratatui::widgets::*;
use regex::Regex;

use super::{App, DetailPane, DiagCategory, LinkPane, Tab};

// ── Main draw ─────────────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tab bar
            Constraint::Min(0),    // Content
            Constraint::Length(1), // Status bar
        ])
        .split(frame.area());

    draw_tabs(frame, app, chunks[0]);

    match app.active_tab {
        Tab::Dashboard => draw_dashboard(frame, app, chunks[1]),
        Tab::Pages => draw_pages(frame, app, chunks[1]),
        Tab::Links => draw_links(frame, app, chunks[1]),
        Tab::Search => draw_search(frame, app, chunks[1]),
        Tab::Diagnostics => draw_diagnostics(frame, app, chunks[1]),
        Tab::PageDetail => draw_page_detail(frame, app, chunks[1]),
        Tab::Graph => draw_graph(frame, app, chunks[1]),
    }

    draw_status_bar(frame, app, chunks[2]);

    // Overlays (drawn last, on top)
    if app.show_help {
        draw_help_overlay(frame);
    }
    if app.switcher_open {
        draw_switcher_overlay(frame, app);
    }
}

// ── Tab bar ───────────────────────────────────────────────────────────────

fn draw_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|tab| Line::from(format!(" {} ", tab.label())))
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" zetl "))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .select(app.active_tab.index());

    frame.render_widget(tabs, area);
}

// ── Status bar ────────────────────────────────────────────────────────────

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let hint = match app.active_tab {
        Tab::Dashboard => "? help | q quit | Tab/Shift+Tab cycle | Ctrl+K switcher",
        Tab::Pages => "type to filter | Enter view page | Esc clear",
        Tab::Links => {
            if app.link_selected_page.is_none() {
                "type page name | Enter select"
            } else {
                "h/l panes | j/k scroll | Enter view page | p detail | g graph | Esc back"
            }
        }
        Tab::Search => {
            if app.search_active {
                "type query | Enter search | Esc cancel"
            } else {
                "/ edit | j/k scroll | Enter view page"
            }
        }
        Tab::Diagnostics => "h/l category | j/k scroll",
        Tab::PageDetail => {
            "h/l panes | j/k wikilinks | Enter follow | PgUp/PgDn scroll | g graph | Backspace back"
        }
        Tab::Graph => {
            "h/l panes | j/k scroll | d toggle depth | Enter navigate | p detail | Backspace back"
        }
    };

    // Breadcrumb trail
    let breadcrumb = if !app.history.is_empty() {
        let recent: Vec<&str> = app
            .history
            .iter()
            .rev()
            .take(3)
            .map(|s| s.as_str())
            .collect();
        let trail: Vec<&str> = recent.into_iter().rev().collect();
        format!(" {} > ", trail.join(" > "))
    } else {
        String::new()
    };

    let bar = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", app.vault_root.display()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(breadcrumb, Style::default().fg(Color::DarkGray)),
        Span::raw("| "),
        Span::styled(hint, Style::default().fg(Color::Cyan)),
    ]))
    .style(Style::default().bg(Color::Black));

    frame.render_widget(bar, area);
}

// ── Dashboard ─────────────────────────────────────────────────────────────

fn draw_dashboard(frame: &mut Frame, app: &App, area: Rect) {
    let stats = app.graph.stats(10);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let stats_items = vec![
        format!("  Pages:                {}", stats.pages),
        format!("  Links:                {}", stats.links),
        format!("  Unique targets:       {}", stats.unique_targets),
        format!("  Dead links:           {}", stats.dead_links),
        format!("  Orphans:              {}", stats.orphans),
        format!("  Connected components: {}", stats.connected_components),
        String::new(),
        format!("  Files scanned:        {}", app.files.len()),
        format!("  Syntax diagnostics:   {}", app.diagnostics.len()),
    ];

    let stats_text: Vec<Line> = stats_items
        .into_iter()
        .map(|s| {
            if s.is_empty() {
                Line::raw("")
            } else {
                Line::raw(s)
            }
        })
        .collect();

    let stats_para = Paragraph::new(stats_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Vault Statistics "),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(stats_para, chunks[0]);

    let rows: Vec<Row> = stats
        .most_linked
        .iter()
        .enumerate()
        .map(|(i, ml)| {
            Row::new(vec![
                Cell::from(format!("{}", i + 1)),
                Cell::from(ml.page.clone()),
                Cell::from(format!("{}", ml.backlink_count)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(20),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec!["#", "Page", "Backlinks"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .bottom_margin(1),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Most Linked Pages "),
    );

    frame.render_widget(table, chunks[1]);
}

// ── Pages ─────────────────────────────────────────────────────────────────

fn draw_pages(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let filter_text = app.page_filter.value();
    let input_widget = Paragraph::new(filter_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Filter (type to search) ")
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(input_widget, chunks[0]);

    frame.set_cursor_position(Position::new(
        chunks[0].x + app.page_filter.visual_cursor() as u16 + 1,
        chunks[0].y + 1,
    ));

    let pages = app.filtered_pages();
    let items: Vec<ListItem> = pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let style = if i == app.page_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("  {page}")).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Pages ({}) ", pages.len())),
    );

    frame.render_widget(list, chunks[1]);
}

// ── Links (enhanced with context) ─────────────────────────────────────────

fn draw_links(frame: &mut Frame, app: &App, area: Rect) {
    if app.link_selected_page.is_none() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        let input_text = app.link_page_input.value();
        let input_widget = Paragraph::new(input_text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Enter page name ")
                .border_style(Style::default().fg(Color::Yellow)),
        );
        frame.render_widget(input_widget, chunks[0]);

        frame.set_cursor_position(Position::new(
            chunks[0].x + app.link_page_input.visual_cursor() as u16 + 1,
            chunks[0].y + 1,
        ));

        let input_lower = app.link_page_input.value().to_lowercase();
        let suggestions: Vec<ListItem> = if input_lower.is_empty() {
            app.page_names
                .iter()
                .take(50)
                .map(|p| ListItem::new(format!("  {p}")))
                .collect()
        } else {
            app.page_names
                .iter()
                .filter(|p| p.to_lowercase().contains(&input_lower))
                .take(50)
                .map(|p| ListItem::new(format!("  {p}")))
                .collect()
        };

        let list = List::new(suggestions).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Suggestions "),
        );
        frame.render_widget(list, chunks[1]);

        return;
    }

    let page = app.link_selected_page.as_ref().unwrap();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new(Line::from(vec![
        Span::raw("  Page: "),
        Span::styled(
            page.clone(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  ({} forward, {} back)",
            app.link_fwd.len(),
            app.link_back.len()
        )),
    ]))
    .block(Block::default().borders(Borders::ALL));

    frame.render_widget(header, outer[0]);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[1]);

    // Forward links with context
    let fwd_border = if app.link_pane == LinkPane::Forward {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let fwd_items: Vec<ListItem> = app
        .link_fwd
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let selected = i == app.link_fwd_selected && app.link_pane == LinkPane::Forward;
            let name_style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let ctx_text = entry.context.as_deref().unwrap_or("").to_string();
            let lines = vec![
                Line::from(vec![
                    Span::styled(format!("  {} ", entry.page), name_style),
                    Span::styled(
                        format!(":{}", entry.line),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::styled(
                    format!("    {ctx_text}"),
                    Style::default().fg(Color::DarkGray),
                ),
            ];
            ListItem::new(lines)
        })
        .collect();

    let fwd_list = List::new(fwd_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Forward Links ({}) ", app.link_fwd.len()))
            .border_style(fwd_border),
    );
    frame.render_widget(fwd_list, panes[0]);

    // Backlinks with context
    let back_border = if app.link_pane == LinkPane::Backward {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let back_items: Vec<ListItem> = app
        .link_back
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let selected = i == app.link_back_selected && app.link_pane == LinkPane::Backward;
            let name_style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let ctx_text = entry.context.as_deref().unwrap_or("").to_string();
            let lines = vec![
                Line::from(vec![
                    Span::styled(format!("  {} ", entry.page), name_style),
                    Span::styled(
                        format!(":{}", entry.line),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::styled(
                    format!("    {ctx_text}"),
                    Style::default().fg(Color::DarkGray),
                ),
            ];
            ListItem::new(lines)
        })
        .collect();

    let back_list = List::new(back_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Backlinks ({}) ", app.link_back.len()))
            .border_style(back_border),
    );
    frame.render_widget(back_list, panes[1]);
}

// ── Search ────────────────────────────────────────────────────────────────

fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let border_style = if app.search_active {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_text = app.search_input.value();
    let input_widget = Paragraph::new(input_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Search (Enter to search, / to edit) ")
            .border_style(border_style),
    );
    frame.render_widget(input_widget, chunks[0]);

    if app.search_active {
        frame.set_cursor_position(Position::new(
            chunks[0].x + app.search_input.visual_cursor() as u16 + 1,
            chunks[0].y + 1,
        ));
    }

    if app.search_results.is_empty() {
        let msg = if app.search_input.value().is_empty() {
            "  Type a query and press Enter to search"
        } else {
            "  No results"
        };
        let para =
            Paragraph::new(msg).block(Block::default().borders(Borders::ALL).title(" Results "));
        frame.render_widget(para, chunks[1]);
    } else {
        let rows: Vec<Row> = app
            .search_results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let style = if i == app.search_selected && !app.search_active {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(r.page.clone()),
                    Cell::from(format!("{}:{}", r.line, r.column)),
                    Cell::from(r.context.clone().unwrap_or_default()),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(25),
                Constraint::Length(10),
                Constraint::Min(30),
            ],
        )
        .header(
            Row::new(vec!["Page", "Loc", "Context"])
                .style(Style::default().add_modifier(Modifier::BOLD))
                .bottom_margin(1),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Results ({}) ", app.search_results.len())),
        );

        frame.render_widget(table, chunks[1]);
    }
}

// ── Diagnostics ───────────────────────────────────────────────────────────

fn draw_diagnostics(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let categories = vec![
        format!(" Dead Links ({}) ", app.dead_links.len()),
        format!(" Orphans ({}) ", app.orphans.len()),
        format!(" Syntax ({}) ", app.diagnostics.len()),
    ];

    let cat_titles: Vec<Line> = categories.into_iter().map(Line::from).collect();
    let selected_idx = match app.diag_category {
        DiagCategory::DeadLinks => 0,
        DiagCategory::Orphans => 1,
        DiagCategory::Syntax => 2,
    };

    let cat_tabs = Tabs::new(cat_titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Diagnostics "),
        )
        .highlight_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .select(selected_idx);

    frame.render_widget(cat_tabs, chunks[0]);

    match app.diag_category {
        DiagCategory::DeadLinks => {
            let rows: Vec<Row> = app
                .dead_links
                .iter()
                .enumerate()
                .map(|(i, dl)| {
                    let style = if i == app.diag_selected {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        Cell::from(dl.source.clone()),
                        Cell::from(format!("{}", dl.line)),
                        Cell::from(dl.target.clone()).style(Style::default().fg(Color::Red)),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Percentage(35),
                    Constraint::Length(8),
                    Constraint::Percentage(50),
                ],
            )
            .header(
                Row::new(vec!["Source", "Line", "Dead Target"])
                    .style(Style::default().add_modifier(Modifier::BOLD))
                    .bottom_margin(1),
            )
            .block(Block::default().borders(Borders::ALL));

            frame.render_widget(table, chunks[1]);
        }
        DiagCategory::Orphans => {
            let rows: Vec<Row> = app
                .orphans
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let style = if i == app.diag_selected {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        Cell::from(o.page.clone()),
                        Cell::from(format!("{}", o.forward_links)),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                rows,
                [Constraint::Percentage(70), Constraint::Percentage(30)],
            )
            .header(
                Row::new(vec!["Orphan Page", "Forward Links"])
                    .style(Style::default().add_modifier(Modifier::BOLD))
                    .bottom_margin(1),
            )
            .block(Block::default().borders(Borders::ALL));

            frame.render_widget(table, chunks[1]);
        }
        DiagCategory::Syntax => {
            let rows: Vec<Row> = app
                .diagnostics
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let level_style = match d.level {
                        crate::types::DiagnosticLevel::Error => Style::default().fg(Color::Red),
                        crate::types::DiagnosticLevel::Warning => {
                            Style::default().fg(Color::Yellow)
                        }
                    };
                    let row_style = if i == app.diag_selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        Cell::from(format!("{:?}", d.level)).style(level_style),
                        Cell::from(d.file.display().to_string()),
                        Cell::from(format!("{}:{}", d.line, d.column)),
                        Cell::from(d.message.clone()),
                    ])
                    .style(row_style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(10),
                    Constraint::Percentage(25),
                    Constraint::Length(10),
                    Constraint::Min(30),
                ],
            )
            .header(
                Row::new(vec!["Level", "File", "Loc", "Message"])
                    .style(Style::default().add_modifier(Modifier::BOLD))
                    .bottom_margin(1),
            )
            .block(Block::default().borders(Borders::ALL));

            frame.render_widget(table, chunks[1]);
        }
    }
}

// ── Page Detail (NEW) ─────────────────────────────────────────────────────

fn draw_page_detail(frame: &mut Frame, app: &App, area: Rect) {
    if app.detail_page.is_none() {
        let msg = Paragraph::new(
            "  No page selected. Use Ctrl+K to find a page, or navigate from Pages/Search/Links.",
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Page Detail "),
        );
        frame.render_widget(msg, area);
        return;
    }

    let page_name = app.detail_page.as_ref().unwrap();

    // Split: content (65%) | sidebar (35%)
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(area);

    // Left: rendered markdown content
    let content_border = if app.detail_pane == DetailPane::Content {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let total_lines = app.detail_content.len();

    // Get selected wikilink info for highlighting
    let selected_wl = app.detail_wikilink_idx.and_then(|idx| {
        app.detail_wikilinks
            .get(idx)
            .map(|wl| (wl.rendered_line, wl.match_on_line))
    });

    let wl_re = Regex::new(r"\[\[([^\[\]]+)\]\]").unwrap();
    let hl_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    // Build all content lines with the specific wikilink highlighted
    let content_lines: Vec<Line> = app
        .detail_content
        .iter()
        .enumerate()
        .map(|(abs_idx, line)| {
            let Some((sel_line, sel_match)) = selected_wl else {
                return line.clone();
            };
            if abs_idx != sel_line {
                return line.clone();
            }
            // Find the byte range of the N-th wikilink in concatenated span text
            let full_text: String = line.iter().map(|s| s.content.as_ref()).collect();
            let Some(hl_range) = wl_re
                .find_iter(&full_text)
                .nth(sel_match)
                .map(|m| (m.start(), m.end()))
            else {
                return line.clone();
            };
            // Rebuild spans, splitting at the highlight range boundaries
            let mut new_spans: Vec<Span<'static>> = Vec::new();
            let mut pos = 0usize;
            for span in line.iter() {
                let span_len = span.content.len();
                let span_start = pos;
                let span_end = pos + span_len;
                if span_end <= hl_range.0 || span_start >= hl_range.1 {
                    // Entirely outside highlight
                    new_spans.push(span.clone());
                } else if span_start >= hl_range.0 && span_end <= hl_range.1 {
                    // Entirely inside highlight
                    new_spans.push(Span::styled(span.content.to_string(), hl_style));
                } else {
                    // Partial overlap — split the span
                    let text = span.content.as_ref();
                    let cut_start = hl_range.0.saturating_sub(span_start);
                    let cut_end = (hl_range.1 - span_start).min(span_len);
                    if cut_start > 0 {
                        new_spans.push(Span::styled(text[..cut_start].to_string(), span.style));
                    }
                    new_spans.push(Span::styled(text[cut_start..cut_end].to_string(), hl_style));
                    if cut_end < span_len {
                        new_spans.push(Span::styled(text[cut_end..].to_string(), span.style));
                    }
                }
                pos = span_end;
            }
            Line::from(new_spans)
        })
        .collect();

    // Show selected wikilink target in title
    let wl_target = app
        .detail_wikilink_idx
        .and_then(|idx| app.detail_wikilinks.get(idx).map(|wl| wl.target.as_str()));

    let scroll_info = if let Some(target) = wl_target {
        format!(
            " {page_name} [{}/{}] → {target} ",
            app.detail_scroll + 1,
            total_lines
        )
    } else if total_lines > 0 {
        format!(" {page_name} [{}/{}] ", app.detail_scroll + 1, total_lines)
    } else {
        format!(" {page_name} ")
    };

    let content_para = Paragraph::new(content_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(scroll_info)
                .border_style(content_border),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll as u16, 0));
    frame.render_widget(content_para, panes[0]);

    // Right: sidebar (properties + backlinks + unlinked mentions)
    let sidebar_border = if app.detail_pane == DetailPane::Sidebar {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut sidebar_lines: Vec<Line> = Vec::new();

    // Frontmatter properties
    if !app.detail_frontmatter.is_empty() {
        sidebar_lines.push(Line::styled(
            "  Properties",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
        for (key, value) in &app.detail_frontmatter {
            sidebar_lines.push(Line::from(vec![
                Span::styled(format!("  {key}: "), Style::default().fg(Color::Cyan)),
                Span::raw(value.clone()),
            ]));
        }
        sidebar_lines.push(Line::raw(""));
    }

    // Backlinks
    sidebar_lines.push(Line::styled(
        format!("  Backlinks ({})", app.detail_backlinks.len()),
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    ));

    for (i, bl) in app.detail_backlinks.iter().enumerate() {
        let is_selected = app.detail_pane == DetailPane::Sidebar && app.detail_sidebar_scroll == i;
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        sidebar_lines.push(Line::styled(format!("  {} :{}", bl.page, bl.line), style));
        if let Some(ref ctx) = bl.context {
            sidebar_lines.push(Line::styled(
                format!("    {ctx}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    // Unlinked mentions
    if !app.detail_unlinked.is_empty() {
        sidebar_lines.push(Line::raw(""));
        sidebar_lines.push(Line::styled(
            format!("  Unlinked Mentions ({})", app.detail_unlinked.len()),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));

        let bl_count = app.detail_backlinks.len();
        for (i, um) in app.detail_unlinked.iter().enumerate() {
            let is_selected =
                app.detail_pane == DetailPane::Sidebar && app.detail_sidebar_scroll == bl_count + i;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            sidebar_lines.push(Line::styled(format!("  {} :{}", um.page, um.line), style));
            sidebar_lines.push(Line::styled(
                format!("    {}", um.context),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    let sidebar_para = Paragraph::new(sidebar_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Sidebar ")
                .border_style(sidebar_border),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(sidebar_para, panes[1]);
}

// ── Local Graph (NEW) ─────────────────────────────────────────────────────

fn draw_graph(frame: &mut Frame, app: &App, area: Rect) {
    if app.graph_page.is_none() {
        let msg = Paragraph::new(
            "  No page selected. Use Ctrl+K to find a page, or press g from Links/Page Detail.",
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Local Graph "),
        );
        frame.render_widget(msg, area);
        return;
    }

    let page = app.graph_page.as_ref().unwrap();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(0)])
        .split(area);

    // Center: page name and stats
    let out_count: usize = app.graph_outgoing.len();
    let in_count: usize = app.graph_incoming.len();
    let depth_label = if app.graph_depth == 1 {
        "1 hop"
    } else {
        "2 hops"
    };

    let header_lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                page.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            format!(
                "  {out_count} outgoing | {in_count} incoming | depth: {depth_label} (d to toggle)"
            ),
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let header = Paragraph::new(header_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Local Graph "),
    );
    frame.render_widget(header, outer[0]);

    // Two columns: outgoing | incoming
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[1]);

    // Outgoing neighbors
    let out_border = if app.graph_pane == LinkPane::Forward {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let out_items: Vec<ListItem> = app
        .graph_outgoing
        .iter()
        .enumerate()
        .flat_map(|(i, n)| {
            let selected = i == app.graph_out_selected && app.graph_pane == LinkPane::Forward;
            let style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };

            let mut items = vec![ListItem::new(Line::styled(
                format!("  -> {}", n.page),
                style,
            ))];

            // Show 2nd-hop children
            for (child, _) in &n.children {
                items.push(ListItem::new(Line::styled(
                    format!("       -> {child}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            items
        })
        .collect();

    let out_list = List::new(out_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Outgoing ({out_count}) "))
            .border_style(out_border),
    );
    frame.render_widget(out_list, panes[0]);

    // Incoming neighbors
    let in_border = if app.graph_pane == LinkPane::Backward {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let in_items: Vec<ListItem> = app
        .graph_incoming
        .iter()
        .enumerate()
        .flat_map(|(i, n)| {
            let selected = i == app.graph_in_selected && app.graph_pane == LinkPane::Backward;
            let style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Magenta)
            };

            let mut items = vec![ListItem::new(Line::styled(
                format!("  <- {}", n.page),
                style,
            ))];

            for (child, _) in &n.children {
                items.push(ListItem::new(Line::styled(
                    format!("       <- {child}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            items
        })
        .collect();

    let in_list = List::new(in_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Incoming ({in_count}) "))
            .border_style(in_border),
    );
    frame.render_widget(in_list, panes[1]);
}

// ── Quick Switcher Overlay (Ctrl+K) ───────────────────────────────────────

fn draw_switcher_overlay(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let overlay = centered_rect(50, 60, area);

    frame.render_widget(Clear, overlay);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(overlay);

    // Input
    let input_text = app.switcher_input.value();
    let input_widget = Paragraph::new(input_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Quick Switcher (Ctrl+K) ")
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Black)),
    );
    frame.render_widget(input_widget, chunks[0]);

    frame.set_cursor_position(Position::new(
        chunks[0].x + app.switcher_input.visual_cursor() as u16 + 1,
        chunks[0].y + 1,
    ));

    // Filtered list
    let pages = app.switcher_filtered();
    let visible = chunks[1].height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = pages
        .iter()
        .take(visible)
        .enumerate()
        .map(|(i, page)| {
            let style = if i == app.switcher_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("  {page}")).style(style)
        })
        .collect();

    let count = pages.len();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {count} pages "))
            .style(Style::default().bg(Color::Black)),
    );
    frame.render_widget(list, chunks[1]);
}

// ── Help overlay ──────────────────────────────────────────────────────────

fn draw_help_overlay(frame: &mut Frame) {
    let area = frame.area();
    let overlay = centered_rect(65, 80, area);

    frame.render_widget(Clear, overlay);

    let help_text = vec![
        Line::styled(
            "  Keybindings",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Yellow),
        ),
        Line::raw(""),
        Line::styled("  Global", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("  q             Quit"),
        Line::raw("  ?             Toggle help"),
        Line::raw("  Tab/Shift+Tab Cycle views"),
        Line::raw("  Ctrl+K        Quick switcher (fuzzy page finder)"),
        Line::raw("  Ctrl+C        Force quit"),
        Line::raw(""),
        Line::styled(
            "  Navigation",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw("  j/Down        Move down / scroll"),
        Line::raw("  k/Up          Move up / scroll"),
        Line::raw("  h/Left        Left pane / prev category"),
        Line::raw("  l/Right       Right pane / next category"),
        Line::raw("  Enter         Select / navigate / follow link"),
        Line::raw("  Esc           Back / clear"),
        Line::raw("  Backspace     Go back in history"),
        Line::raw("  PgUp/PgDn     Scroll page content"),
        Line::raw(""),
        Line::styled(
            "  Page Detail",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw("  h/l           Switch content/sidebar panes"),
        Line::raw("  g             Open local graph for this page"),
        Line::raw(""),
        Line::styled(
            "  Local Graph",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw("  d             Toggle depth (1 hop / 2 hops)"),
        Line::raw("  p             Open page detail"),
        Line::raw("  Enter         Re-center graph on selected neighbor"),
        Line::raw(""),
        Line::styled("  Links", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("  p             Open page detail for current page"),
        Line::raw("  g             Open local graph for current page"),
        Line::raw(""),
        Line::styled("  Search", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("  /             Focus search input"),
        Line::raw("  Enter         Execute search / view selected page"),
        Line::raw(""),
        Line::styled(
            "  Press any key to close",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let help = Paragraph::new(help_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Help ")
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Black)),
    );

    frame.render_widget(help, overlay);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
