use crate::{ArtistTracksIterator, LastFmClient, Result, ScrobbleEdit, Track};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    BrowseTracks,
    EditTrack,
    Loading,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct TrackEditorApp {
    pub artist: String,
    pub tracks: Vec<Track>,
    pub selected_track_index: usize,
    pub mode: AppMode,
    pub list_state: ListState,
    pub edit_field: String,
    pub edit_buffer: String,
    pub loading_message: String,
    pub status_message: String,
    pub current_edit: Option<ScrobbleEdit>,
    pub page: u32,
    pub has_more_pages: bool,
}

impl TrackEditorApp {
    pub fn new(artist: String) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            artist,
            tracks: Vec::new(),
            selected_track_index: 0,
            mode: AppMode::Loading,
            list_state,
            edit_field: String::new(),
            edit_buffer: String::new(),
            loading_message: "Loading tracks...".to_string(),
            status_message: String::new(),
            current_edit: None,
            page: 1,
            has_more_pages: true,
        }
    }

    pub async fn load_tracks(&mut self, client: &mut LastFmClient) -> Result<()> {
        self.mode = AppMode::Loading;
        self.loading_message = format!("Loading tracks for {}...", self.artist);

        // Load first page of tracks
        let mut iterator = ArtistTracksIterator::new(client, self.artist.clone());
        let track_page = iterator.next_page().await?;

        if let Some(page) = track_page {
            self.tracks = page.tracks;
            self.has_more_pages = page.has_next_page;
            self.page = page.page_number;
        } else {
            self.tracks = Vec::new();
            self.has_more_pages = false;
            self.page = 1;
        }

        if self.tracks.is_empty() {
            self.mode = AppMode::Error("No tracks found for this artist".to_string());
        } else {
            self.mode = AppMode::BrowseTracks;
            self.selected_track_index = 0;
            self.list_state.select(Some(0));
        }

        Ok(())
    }

    pub async fn load_more_tracks(&mut self, client: &mut LastFmClient) -> Result<()> {
        if !self.has_more_pages {
            return Ok(());
        }

        self.mode = AppMode::Loading;
        self.loading_message = format!("Loading more tracks (page {})...", self.page + 1);

        let mut iterator = ArtistTracksIterator::new(client, self.artist.clone());
        // Skip to the next page
        for _ in 0..self.page {
            iterator.next_page().await?;
        }
        let track_page = iterator.next_page().await?;

        if let Some(page) = track_page {
            self.tracks.extend(page.tracks);
            self.has_more_pages = page.has_next_page;
            self.page = page.page_number;
        }

        self.mode = AppMode::BrowseTracks;
        Ok(())
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        match self.mode {
            AppMode::BrowseTracks => self.handle_browse_keys(key),
            AppMode::EditTrack => self.handle_edit_keys(key),
            AppMode::Loading => false, // No input during loading
            AppMode::Error(_) => {
                // Press any key to return to browse mode (if we have tracks)
                if !self.tracks.is_empty() {
                    self.mode = AppMode::BrowseTracks;
                }
                false
            }
        }
    }

    fn handle_browse_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return true, // Quit
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_track_index > 0 {
                    self.selected_track_index -= 1;
                    self.list_state.select(Some(self.selected_track_index));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_track_index < self.tracks.len().saturating_sub(1) {
                    self.selected_track_index += 1;
                    self.list_state.select(Some(self.selected_track_index));
                }
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(track) = self.tracks.get(self.selected_track_index) {
                    self.start_edit_mode(track.clone());
                }
            }
            KeyCode::Char('n') => {
                // Load next page
                return false; // Signal that we need to load more tracks
            }
            _ => {}
        }
        false
    }

    fn handle_edit_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return true, // Quit
            KeyCode::Esc => {
                self.mode = AppMode::BrowseTracks;
                self.edit_buffer.clear();
                self.current_edit = None;
            }
            KeyCode::Enter => {
                // Save edit - signal that we need to perform the edit
                return false;
            }
            KeyCode::Backspace => {
                self.edit_buffer.pop();
                if let Some(ref mut edit) = self.current_edit {
                    edit.track_name = self.edit_buffer.clone();
                }
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match c {
                        'c' => return true, // Ctrl+C to quit
                        _ => {}
                    }
                } else {
                    self.edit_buffer.push(c);
                    if let Some(ref mut edit) = self.current_edit {
                        edit.track_name = self.edit_buffer.clone();
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn start_edit_mode(&mut self, track: Track) {
        self.mode = AppMode::Loading;
        self.loading_message = format!("Loading edit form for '{}'...", track.name);
        self.edit_buffer = track.name.clone();
        self.edit_field = track.name;
    }

    pub async fn load_edit_form(&mut self, client: &mut LastFmClient) -> Result<()> {
        if let Some(track) = self.tracks.get(self.selected_track_index) {
            match client
                .load_edit_form_values(&track.name, &track.artist)
                .await
            {
                Ok(edit) => {
                    self.current_edit = Some(edit);
                    self.edit_buffer = track.name.clone();
                    self.mode = AppMode::EditTrack;
                    self.status_message =
                        "Editing track name. Press Enter to save, Esc to cancel.".to_string();
                }
                Err(e) => {
                    self.mode = AppMode::Error(format!("Failed to load edit form: {}", e));
                }
            }
        }
        Ok(())
    }

    pub async fn save_edit(&mut self, client: &mut LastFmClient) -> Result<()> {
        if let Some(ref edit) = self.current_edit {
            self.mode = AppMode::Loading;
            self.loading_message = format!(
                "Saving edit: '{}' -> '{}'...",
                edit.track_name_original, edit.track_name
            );

            match client.edit_scrobble(edit).await {
                Ok(response) => {
                    if response.success {
                        self.status_message = format!(
                            "✅ Successfully edited '{}' to '{}'",
                            edit.track_name_original, edit.track_name
                        );

                        // Update the track in our list
                        if let Some(track) = self.tracks.get_mut(self.selected_track_index) {
                            track.name = edit.track_name.clone();
                        }
                    } else {
                        self.status_message = format!(
                            "❌ Edit failed: {}",
                            response
                                .message
                                .unwrap_or_else(|| "Unknown error".to_string())
                        );
                    }
                }
                Err(e) => {
                    self.status_message = format!("❌ Edit error: {}", e);
                }
            }

            self.mode = AppMode::BrowseTracks;
            self.current_edit = None;
            self.edit_buffer.clear();
        }
        Ok(())
    }
}

pub fn render_ui(f: &mut Frame, app: &TrackEditorApp) {
    let size = f.area();

    match &app.mode {
        AppMode::Loading => {
            render_loading_screen(f, size, &app.loading_message);
        }
        AppMode::Error(error) => {
            render_error_screen(f, size, error);
        }
        AppMode::BrowseTracks => {
            render_browse_screen(f, app, size);
        }
        AppMode::EditTrack => {
            render_edit_screen(f, app, size);
        }
    }
}

fn render_loading_screen(f: &mut Frame, area: Rect, message: &str) {
    let block = Block::default()
        .title("Loading")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(format!("{}\n\nPlease wait...", message))
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    let centered = centered_rect(60, 20, area);
    f.render_widget(paragraph, centered);
}

fn render_error_screen(f: &mut Frame, area: Rect, error: &str) {
    let block = Block::default()
        .title("Error")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let paragraph = Paragraph::new(format!("{}\n\nPress any key to continue...", error))
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    let centered = centered_rect(80, 30, area);
    f.render_widget(paragraph, centered);
}

fn render_browse_screen(f: &mut Frame, app: &TrackEditorApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Track list
            Constraint::Length(3), // Status
            Constraint::Length(4), // Help
        ])
        .split(area);

    // Title
    let title = Paragraph::new(format!("Last.fm Track Editor - {}", app.artist))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Track list
    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let line = format!("{:3}. {} ({})", i + 1, track.name, track.playcount);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    "Tracks (Page {} - {} tracks)",
                    app.page,
                    app.tracks.len()
                ))
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, chunks[1], &mut app.list_state.clone());

    // Status message
    let status = Paragraph::new(app.status_message.as_str())
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: true });
    f.render_widget(status, chunks[2]);

    // Help
    let help_text = "↑/k: Up  ↓/j: Down  Enter/e: Edit  n: Next page  q: Quit";
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .alignment(Alignment::Center);
    f.render_widget(help, chunks[3]);
}

fn render_edit_screen(f: &mut Frame, app: &TrackEditorApp, area: Rect) {
    // Create overlay for edit dialog
    let centered = centered_rect(80, 50, area);

    // Clear the background
    f.render_widget(Clear, centered);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(3), // Original name
            Constraint::Length(3), // New name input
            Constraint::Min(0),    // Spacer
            Constraint::Length(3), // Help
        ])
        .split(centered);

    // Edit dialog title
    let title = Paragraph::new("Edit Track Name")
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Original track name
    let original = if let Some(ref edit) = app.current_edit {
        format!("Original: {}", edit.track_name_original)
    } else {
        "Original: Loading...".to_string()
    };
    let original_widget = Paragraph::new(original)
        .block(Block::default().borders(Borders::ALL).title("Current"))
        .wrap(Wrap { trim: true });
    f.render_widget(original_widget, chunks[1]);

    // New track name input
    let new_name = Paragraph::new(app.edit_buffer.as_str())
        .block(Block::default().borders(Borders::ALL).title("New Name"))
        .wrap(Wrap { trim: true });
    f.render_widget(new_name, chunks[2]);

    // Help
    let help_text = "Type to edit  Enter: Save  Esc: Cancel  Ctrl+C/q: Quit";
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .alignment(Alignment::Center);
    f.render_widget(help, chunks[4]);
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

pub async fn run_track_editor(mut client: LastFmClient, artist: String) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = TrackEditorApp::new(artist);

    // Load initial tracks
    app.load_tracks(&mut client).await?;

    let mut should_quit = false;
    while !should_quit {
        // Render UI
        terminal.draw(|f| render_ui(f, &app))?;

        // Handle events
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                let quit_requested = app.handle_key_event(key);
                if quit_requested {
                    should_quit = true;
                    continue;
                }

                // Handle special actions that require async operations
                match (&app.mode, key.code) {
                    (AppMode::BrowseTracks, KeyCode::Char('n')) => {
                        app.load_more_tracks(&mut client).await?;
                    }
                    (AppMode::Loading, _) if app.loading_message.contains("Loading edit form") => {
                        app.load_edit_form(&mut client).await?;
                    }
                    (AppMode::EditTrack, KeyCode::Enter) => {
                        app.save_edit(&mut client).await?;
                    }
                    _ => {}
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
