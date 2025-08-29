use chrono::{Datelike, Local, Timelike};
use clap::{Arg, Command};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgTimestamp {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub day_name: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgClockEntry {
    pub start: OrgTimestamp,
    pub end: Option<OrgTimestamp>,
    pub duration: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgLogbook {
    pub clock_entries: Vec<OrgClockEntry>,
    pub raw_content: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgPlanning {
    pub scheduled: Option<OrgTimestamp>,
    pub deadline: Option<OrgTimestamp>,
    pub closed: Option<OrgTimestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgNote {
    pub level: usize,
    pub status: Option<String>,
    pub title: String,
    pub labels: Vec<String>,
    pub content: String,
    pub children: Vec<OrgNote>,
    pub planning: Option<OrgPlanning>,
    pub logbook: Option<OrgLogbook>,
}

impl OrgNote {
    pub fn new(level: usize, title: String) -> Self {
        Self {
            level,
            status: None,
            title,
            labels: Vec::new(),
            content: String::new(),
            children: Vec::new(),
            planning: None,
            logbook: None,
        }
    }
}

pub struct OrgParser {
    lines: Vec<String>,
    current_line: usize,
}

impl OrgParser {
    pub fn new(content: &str) -> Self {
        Self {
            lines: content.lines().map(|s| s.to_string()).collect(),
            current_line: 0,
        }
    }

    pub fn parse(&mut self) -> Vec<OrgNote> {
        let mut notes = Vec::new();

        while self.current_line < self.lines.len() {
            let line = &self.lines[self.current_line];

            if let Some(level) = self.count_asterisks(line) {
                if let Some(note) = self.parse_note(level) {
                    notes.push(note);
                }
            } else {
                self.current_line += 1;
            }
        }

        notes
    }

    fn count_asterisks(&self, line: &str) -> Option<usize> {
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') {
            let count = trimmed.chars().take_while(|&c| c == '*').count();
            if count > 0 && trimmed.chars().nth(count) == Some(' ') {
                return Some(count);
            }
        }
        None
    }

    fn parse_note(&mut self, level: usize) -> Option<OrgNote> {
        if self.current_line >= self.lines.len() {
            return None;
        }

        let line = &self.lines[self.current_line];
        let header_content = self.extract_header_content(line, level);

        let (status, title, labels) = self.parse_header_parts(&header_content);

        let mut note = OrgNote::new(level, title);
        note.status = status;
        note.labels = labels;

        self.current_line += 1;

        // Collect content until next heading of same or higher level
        let mut content_lines = Vec::new();
        let mut child_notes = Vec::new();

        while self.current_line < self.lines.len() {
            let line = &self.lines[self.current_line];

            if let Some(next_level) = self.count_asterisks(line) {
                if next_level <= level {
                    // Same or higher level heading, stop collecting content
                    break;
                } else {
                    // Child heading, parse it as a child note
                    if let Some(child_note) = self.parse_note(next_level) {
                        child_notes.push(child_note);
                    }
                }
            } else {
                // Regular content line
                content_lines.push(line.clone());
                self.current_line += 1;
            }
        }

        let content_text = content_lines.join("\n");
        let (cleaned_content, planning, logbook) = self.parse_time_elements(&content_text);

        note.content = cleaned_content;
        note.planning = planning;
        note.logbook = logbook;
        note.children = child_notes;

        Some(note)
    }

    fn extract_header_content(&self, line: &str, level: usize) -> String {
        let trimmed = line.trim_start();
        // Skip the asterisks and the space after them
        trimmed.chars().skip(level + 1).collect()
    }

    fn parse_header_parts(&self, header: &str) -> (Option<String>, String, Vec<String>) {
        let trimmed = header.trim();

        // Extract labels (org-mode tags at the end, starting with :)
        let mut labels = Vec::new();
        let mut content = trimmed;

        // Find the last space followed by a colon (start of tags section)
        if let Some(tag_start) = trimmed.rfind(char::is_whitespace) {
            let potential_tags = &trimmed[tag_start..].trim_start();
            if potential_tags.starts_with(':')
                && potential_tags.ends_with(':')
                && potential_tags.len() > 2
            {
                // Extract tags between colons
                let tags_content = &potential_tags[1..potential_tags.len() - 1];
                labels = tags_content
                    .split(':')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                content = trimmed[..tag_start].trim();
            }
        }

        // Extract status (first word if it's uppercase)
        let words: Vec<&str> = content.split_whitespace().collect();
        let mut status = None;
        let mut title_start = 0;

        if let Some(first_word) = words.first() {
            if first_word
                .chars()
                .all(|c| c.is_uppercase() || !c.is_alphabetic())
                && first_word.len() > 0
            {
                status = Some(first_word.to_string());
                title_start = 1;
            }
        }

        let title = words[title_start..].join(" ");

        (status, title, labels)
    }

    fn parse_time_elements(
        &self,
        content: &str,
    ) -> (String, Option<OrgPlanning>, Option<OrgLogbook>) {
        let lines: Vec<&str> = content.lines().collect();
        let mut cleaned_lines = Vec::new();
        let mut planning = OrgPlanning {
            scheduled: None,
            deadline: None,
            closed: None,
        };
        let mut logbook = None;
        let mut in_logbook = false;
        let mut logbook_lines = Vec::new();
        let mut clock_entries = Vec::new();

        for line in lines {
            let trimmed = line.trim();

            // Check for logbook start/end
            if trimmed == ":LOGBOOK:" {
                in_logbook = true;
                continue;
            } else if trimmed == ":END:" && in_logbook {
                in_logbook = false;
                logbook = Some(OrgLogbook {
                    clock_entries: clock_entries.clone(),
                    raw_content: logbook_lines.clone(),
                });
                logbook_lines.clear();
                continue;
            }

            if in_logbook {
                logbook_lines.push(line.to_string());
                if let Some(clock_entry) = self.parse_clock_line(line) {
                    clock_entries.push(clock_entry);
                }
                continue;
            }

            // Check for planning keywords
            if let Some(timestamp) = self.extract_planning_timestamp(line, "SCHEDULED:") {
                planning.scheduled = Some(timestamp);
                continue;
            } else if let Some(timestamp) = self.extract_planning_timestamp(line, "DEADLINE:") {
                planning.deadline = Some(timestamp);
                continue;
            } else if let Some(timestamp) = self.extract_planning_timestamp(line, "CLOSED:") {
                planning.closed = Some(timestamp);
                continue;
            }

            cleaned_lines.push(line);
        }

        let has_planning = planning.scheduled.is_some()
            || planning.deadline.is_some()
            || planning.closed.is_some();
        let final_planning = if has_planning { Some(planning) } else { None };

        (cleaned_lines.join("\n"), final_planning, logbook)
    }

    fn extract_planning_timestamp(&self, line: &str, keyword: &str) -> Option<OrgTimestamp> {
        if let Some(pos) = line.find(keyword) {
            let after_keyword = &line[pos + keyword.len()..].trim();
            self.parse_timestamp_from_text(after_keyword)
        } else {
            None
        }
    }

    fn parse_clock_line(&self, line: &str) -> Option<OrgClockEntry> {
        let trimmed = line.trim();
        if !trimmed.starts_with("CLOCK:") {
            return None;
        }

        let clock_content = &trimmed[6..].trim();

        // Parse format: [start]--[end] => duration
        if let Some(arrow_pos) = clock_content.find("=>") {
            let time_part = &clock_content[..arrow_pos].trim();
            let duration_part = clock_content[arrow_pos + 2..].trim();

            if let Some(dash_pos) = time_part.find("--") {
                let start_part = &time_part[..dash_pos].trim();
                let end_part = &time_part[dash_pos + 2..].trim();

                if let (Some(start), Some(end)) = (
                    self.parse_timestamp_from_text(start_part),
                    self.parse_timestamp_from_text(end_part),
                ) {
                    return Some(OrgClockEntry {
                        start,
                        end: Some(end),
                        duration: Some(duration_part.to_string()),
                        raw: line.to_string(),
                    });
                }
            }
        } else if let Some(timestamp) = self.parse_timestamp_from_text(clock_content) {
            // Single timestamp (clock in, no clock out yet)
            return Some(OrgClockEntry {
                start: timestamp,
                end: None,
                duration: None,
                raw: line.to_string(),
            });
        }

        None
    }

    fn parse_timestamp_from_text(&self, text: &str) -> Option<OrgTimestamp> {
        // Handle both [timestamp] and <timestamp> formats
        let content = if text.starts_with('[') && text.ends_with(']') {
            &text[1..text.len() - 1]
        } else if text.starts_with('<') && text.ends_with('>') {
            &text[1..text.len() - 1]
        } else {
            text
        };

        // Parse format like: "2024-01-01 Mon 10:00" or "2023-03-29 Ср"
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }

        // Parse date part (YYYY-MM-DD)
        let date_parts: Vec<&str> = parts[0].split('-').collect();
        if date_parts.len() != 3 {
            return None;
        }

        let year = date_parts[0].parse::<u32>().ok()?;
        let month = date_parts[1].parse::<u32>().ok()?;
        let day = date_parts[2].parse::<u32>().ok()?;

        let day_name = if parts.len() > 1 {
            Some(parts[1].to_string())
        } else {
            None
        };

        // Parse time part if present (HH:MM)
        let (hour, minute) = if parts.len() > 2 {
            let time_parts: Vec<&str> = parts[2].split(':').collect();
            if time_parts.len() == 2 {
                let h = time_parts[0].parse::<u32>().ok();
                let m = time_parts[1].parse::<u32>().ok();
                (h, m)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Some(OrgTimestamp {
            year,
            month,
            day,
            hour,
            minute,
            day_name,
            raw: text.to_string(),
        })
    }
}

impl OrgTimestamp {
    pub fn to_date_string(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn to_datetime_string(&self) -> String {
        if let (Some(hour), Some(minute)) = (self.hour, self.minute) {
            format!("{} {:02}:{:02}", self.to_date_string(), hour, minute)
        } else {
            self.to_date_string()
        }
    }
}

impl OrgClockEntry {
    pub fn parse_duration_minutes(&self) -> Option<u32> {
        self.duration.as_ref().and_then(|d| {
            let parts: Vec<&str> = d.trim().split(':').collect();
            if parts.len() == 2 {
                let hours = parts[0].parse::<u32>().ok()?;
                let minutes = parts[1].parse::<u32>().ok()?;
                Some(hours * 60 + minutes)
            } else {
                None
            }
        })
    }

    pub fn format_duration(&self) -> String {
        if let Some(duration) = &self.duration {
            format!(
                "{} ({})",
                duration,
                if let Some(mins) = self.parse_duration_minutes() {
                    format!("{} minutes", mins)
                } else {
                    "duration".to_string()
                }
            )
        } else {
            "running".to_string()
        }
    }
}

impl OrgLogbook {
    pub fn total_minutes(&self) -> u32 {
        self.clock_entries
            .iter()
            .filter_map(|entry| entry.parse_duration_minutes())
            .sum()
    }

    pub fn format_total_time(&self) -> String {
        let total_mins = self.total_minutes();
        let hours = total_mins / 60;
        let minutes = total_mins % 60;
        format!("{}h {}m", hours, minutes)
    }
}

fn print_time_summary(notes: &[OrgNote]) {
    let mut total_tracked_minutes = 0;
    let mut completed_tasks = 0;
    let mut active_tasks = 0;
    let mut scheduled_tasks = 0;
    let mut overdue_tasks = 0;

    collect_time_stats(
        notes,
        &mut total_tracked_minutes,
        &mut completed_tasks,
        &mut active_tasks,
        &mut scheduled_tasks,
        &mut overdue_tasks,
    );

    println!("Time Tracking Summary:");
    println!("---------------------");
    println!(
        "Total tracked time: {}h {}m",
        total_tracked_minutes / 60,
        total_tracked_minutes % 60
    );
    println!("Completed tasks: {}", completed_tasks);
    println!("Active tasks: {}", active_tasks);
    println!("Scheduled tasks: {}", scheduled_tasks);
    if overdue_tasks > 0 {
        println!("⚠️  Overdue tasks: {}", overdue_tasks);
    }
    println!();
}

fn collect_time_stats(
    notes: &[OrgNote],
    total_minutes: &mut u32,
    completed: &mut u32,
    active: &mut u32,
    scheduled: &mut u32,
    overdue: &mut u32,
) {
    for note in notes {
        if let Some(logbook) = &note.logbook {
            *total_minutes += logbook.total_minutes();
        }

        match &note.status {
            Some(status) if status == "DONE" => *completed += 1,
            Some(status) if status == "TODO" || status == "IN-PROGRESS" => *active += 1,
            _ => {}
        }

        if let Some(planning) = &note.planning {
            if planning.scheduled.is_some() {
                *scheduled += 1;
            }

            // Simple overdue check (tasks with deadlines in the past)
            if let Some(deadline) = &planning.deadline {
                if deadline.year < 2024 || (deadline.year == 2024 && deadline.month < 12) {
                    *overdue += 1;
                }
            }
        }

        collect_time_stats(
            &note.children,
            total_minutes,
            completed,
            active,
            scheduled,
            overdue,
        );
    }
}

#[derive(Clone)]
enum Focus {
    Left,
    Right,
}

#[derive(Clone, PartialEq)]
enum EditMode {
    None,
    Status,
    Title,
    Labels,
    Content,
    Scheduled,
    Deadline,
    Closed,
}

struct App {
    notes: Vec<OrgNote>,
    flat_notes: Vec<(usize, String)>, // (index in notes tree, display string)
    selected_note_idx: usize,
    selected_field_idx: usize,
    focus: Focus,
    edit_mode: EditMode,
    edit_buffer: String,
    list_state: ListState,
    file_path: String,
    modified: bool,
    status_message: String,
}

impl App {
    fn new(notes: Vec<OrgNote>, file_path: String) -> Self {
        let flat_notes = Self::flatten_notes(&notes);
        let mut list_state = ListState::default();
        if !flat_notes.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            notes,
            flat_notes,
            selected_note_idx: 0,
            selected_field_idx: 0,
            focus: Focus::Left,
            edit_mode: EditMode::None,
            edit_buffer: String::new(),
            list_state,
            file_path,
            modified: false,
            status_message: "Press Tab to switch panels, Enter to edit, q to quit".to_string(),
        }
    }

    fn flatten_notes(notes: &[OrgNote]) -> Vec<(usize, String)> {
        let mut flat = Vec::new();
        Self::flatten_recursive(notes, &mut flat, 0);
        flat
    }

    fn flatten_recursive(notes: &[OrgNote], flat: &mut Vec<(usize, String)>, depth: usize) {
        for (_idx, note) in notes.iter().enumerate() {
            let indent = "  ".repeat(depth);
            let status = if let Some(s) = &note.status {
                format!("{} ", s)
            } else {
                String::new()
            };
            let display = format!(
                "{}{}*{} {}{}",
                indent,
                if depth > 0 { "" } else { "" },
                "*".repeat(note.level.saturating_sub(depth)),
                status,
                note.title
            );
            let flat_idx = flat.len(); // Use sequential index instead of tree index
            flat.push((flat_idx, display));
            Self::flatten_recursive(&note.children, flat, depth + 1);
        }
    }

    fn get_selected_note(&self) -> Option<&OrgNote> {
        if self.flat_notes.is_empty() {
            return None;
        }

        Self::find_note_by_flat_index(&self.notes, self.selected_note_idx, &mut 0)
    }

    fn get_selected_note_mut(&mut self) -> Option<&mut OrgNote> {
        if self.flat_notes.is_empty() {
            return None;
        }

        let target_idx = self.selected_note_idx;
        Self::find_note_by_flat_index_mut(&mut self.notes, target_idx, &mut 0)
    }

    fn find_note_by_flat_index<'a>(
        notes: &'a [OrgNote],
        target_idx: usize,
        current_idx: &mut usize,
    ) -> Option<&'a OrgNote> {
        for note in notes.iter() {
            if *current_idx == target_idx {
                return Some(note);
            }
            *current_idx += 1;

            if let Some(found) =
                Self::find_note_by_flat_index(&note.children, target_idx, current_idx)
            {
                return Some(found);
            }
        }
        None
    }

    fn find_note_by_flat_index_mut<'a>(
        notes: &'a mut [OrgNote],
        target_idx: usize,
        current_idx: &mut usize,
    ) -> Option<&'a mut OrgNote> {
        for note in notes.iter_mut() {
            if *current_idx == target_idx {
                return Some(note);
            }
            *current_idx += 1;

            if let Some(found) =
                Self::find_note_by_flat_index_mut(&mut note.children, target_idx, current_idx)
            {
                return Some(found);
            }
        }
        None
    }

    fn add_note(&mut self) {
        let new_note = OrgNote::new(1, "New Note".to_string());
        self.notes.push(new_note);
        self.flat_notes = Self::flatten_notes(&self.notes);
        self.selected_note_idx = self.flat_notes.len() - 1;
        self.list_state.select(Some(self.selected_note_idx));
        self.modified = true;
    }

    fn delete_selected_note(&mut self) {
        if !self.flat_notes.is_empty() {
            // Find and remove the note from the tree structure
            Self::remove_note_by_flat_index(&mut self.notes, self.selected_note_idx, &mut 0);
            self.flat_notes = Self::flatten_notes(&self.notes);

            if self.selected_note_idx >= self.flat_notes.len() && !self.flat_notes.is_empty() {
                self.selected_note_idx = self.flat_notes.len() - 1;
            }

            if self.flat_notes.is_empty() {
                self.list_state.select(None);
            } else {
                self.list_state.select(Some(self.selected_note_idx));
            }

            self.modified = true;
        }
    }

    fn remove_note_by_flat_index(
        notes: &mut Vec<OrgNote>,
        target_idx: usize,
        current_idx: &mut usize,
    ) -> bool {
        let mut i = 0;
        while i < notes.len() {
            if *current_idx == target_idx {
                notes.remove(i);
                return true;
            }
            *current_idx += 1;

            if Self::remove_note_by_flat_index(&mut notes[i].children, target_idx, current_idx) {
                return true;
            }
            i += 1;
        }
        false
    }

    fn clock_in(&mut self) {
        if let Some(note) = self.get_selected_note_mut() {
            let now = Local::now();
            let timestamp = OrgTimestamp {
                year: now.year() as u32,
                month: now.month(),
                day: now.day(),
                hour: Some(now.hour()),
                minute: Some(now.minute()),
                day_name: Some(now.format("%a").to_string()),
                raw: now.format("[%Y-%m-%d %a %H:%M]").to_string(),
            };

            let clock_entry = OrgClockEntry {
                start: timestamp,
                end: None,
                duration: None,
                raw: now.format("CLOCK: [%Y-%m-%d %a %H:%M]").to_string(),
            };

            if let Some(logbook) = &mut note.logbook {
                logbook.clock_entries.push(clock_entry);
            } else {
                note.logbook = Some(OrgLogbook {
                    clock_entries: vec![clock_entry],
                    raw_content: Vec::new(),
                });
            }

            self.modified = true;
        }
    }

    fn clock_out(&mut self) {
        if let Some(note) = self.get_selected_note_mut() {
            if let Some(logbook) = &mut note.logbook {
                // Find the oldest running clock entry
                for entry in &mut logbook.clock_entries {
                    if entry.end.is_none() {
                        let now = Local::now();
                        let end_timestamp = OrgTimestamp {
                            year: now.year() as u32,
                            month: now.month(),
                            day: now.day(),
                            hour: Some(now.hour()),
                            minute: Some(now.minute()),
                            day_name: Some(now.format("%a").to_string()),
                            raw: now.format("[%Y-%m-%d %a %H:%M]").to_string(),
                        };

                        entry.end = Some(end_timestamp);
                        // Calculate duration (simplified)
                        let start_time =
                            entry.start.hour.unwrap_or(0) * 60 + entry.start.minute.unwrap_or(0);
                        let end_time = now.hour() * 60 + now.minute();
                        let duration_mins = if end_time >= start_time {
                            end_time - start_time
                        } else {
                            (24 * 60) - start_time + end_time
                        };

                        entry.duration =
                            Some(format!("{}:{:02}", duration_mins / 60, duration_mins % 60));
                        entry.raw = format!(
                            "{}--{} =>  {}",
                            entry.start.raw,
                            now.format("[%Y-%m-%d %a %H:%M]"),
                            entry.duration.as_ref().unwrap()
                        );

                        self.modified = true;
                        break;
                    }
                }
            }
        }
    }

    fn set_current_time(&mut self, field: &str) {
        if let Some(note) = self.get_selected_note_mut() {
            let now = Local::now();
            let timestamp = OrgTimestamp {
                year: now.year() as u32,
                month: now.month(),
                day: now.day(),
                hour: Some(now.hour()),
                minute: Some(now.minute()),
                day_name: Some(now.format("%a").to_string()),
                raw: format!(
                    "<{}-{:02}-{:02} {} {:02}:{:02}>",
                    now.year(),
                    now.month(),
                    now.day(),
                    now.format("%a"),
                    now.hour(),
                    now.minute(),
                ),
            };

            if note.planning.is_none() {
                note.planning = Some(OrgPlanning {
                    scheduled: None,
                    deadline: None,
                    closed: None,
                });
            }

            if let Some(planning) = &mut note.planning {
                match field {
                    "scheduled" => planning.scheduled = Some(timestamp),
                    "deadline" => planning.deadline = Some(timestamp),
                    "closed" => planning.closed = Some(timestamp),
                    _ => {}
                }
            }

            self.modified = true;
        }
    }

    fn save_to_file(&self) -> io::Result<()> {
        let content = self.serialize_to_org_format();
        fs::write(&self.file_path, content)
    }

    fn serialize_to_org_format(&self) -> String {
        let mut output = String::new();

        for note in &self.notes {
            Self::serialize_note(&mut output, note);
        }

        output
    }

    fn serialize_note(output: &mut String, note: &OrgNote) {
        // Write heading
        let stars = "*".repeat(note.level);
        let status = if let Some(s) = &note.status {
            format!(" {}", s)
        } else {
            String::new()
        };
        let labels = if !note.labels.is_empty() {
            format!(" :{}:", note.labels.join(":"))
        } else {
            String::new()
        };

        output.push_str(&format!("{}{} {}{}\n", stars, status, note.title, labels));

        // Write planning
        if let Some(planning) = &note.planning {
            if let Some(scheduled) = &planning.scheduled {
                output.push_str(&format!("SCHEDULED: {}\n", scheduled.raw));
            }
            if let Some(deadline) = &planning.deadline {
                output.push_str(&format!("DEADLINE: {}\n", deadline.raw));
            }
            if let Some(closed) = &planning.closed {
                output.push_str(&format!("CLOSED: {}\n", closed.raw));
            }
        }

        // Write logbook
        if let Some(logbook) = &note.logbook {
            if !logbook.clock_entries.is_empty() {
                output.push_str(":LOGBOOK:\n");
                for entry in &logbook.clock_entries {
                    output.push_str(&format!("{}\n", entry.raw));
                }
                output.push_str(":END:\n");
            }
        }

        // Write content
        if !note.content.trim().is_empty() {
            output.push_str(&format!("{}\n", note.content));
        }

        output.push('\n');

        // Write children
        for child in &note.children {
            Self::serialize_note(output, child);
        }
    }
}

fn run_tui(notes: Vec<OrgNote>, file_path: String) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {}", e))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|e| format!("Failed to setup terminal: {}", e))?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|e| format!("Failed to create terminal: {}", e))?;

    let mut app = App::new(notes, file_path);
    let res = run_app(&mut terminal, &mut app);

    // Cleanup terminal
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();

    Ok(res?)
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        match event::read() {
            Ok(Event::Key(key)) => {
                match app.edit_mode {
                    EditMode::None => {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('q'), KeyModifiers::NONE) => return Ok(()),
                            (KeyCode::Tab, KeyModifiers::NONE) => {
                                app.focus = match app.focus {
                                    Focus::Left => Focus::Right,
                                    Focus::Right => Focus::Left,
                                };
                            }
                            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                                if let Err(_) = app.save_to_file() {
                                    // Handle save error
                                } else {
                                    app.modified = false;
                                }
                            }
                            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                                app.add_note();
                            }
                            (KeyCode::Delete, KeyModifiers::NONE) => {
                                app.delete_selected_note();
                            }
                            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                                app.clock_in();
                            }
                            (KeyCode::Char('o'), KeyModifiers::NONE) => {
                                app.clock_out();
                            }
                            (KeyCode::Char('k'), KeyModifiers::NONE) => {
                                app.set_current_time("scheduled");
                            }
                            (KeyCode::Char('l'), KeyModifiers::NONE) => {
                                app.set_current_time("deadline");
                            }
                            (KeyCode::Char('='), KeyModifiers::NONE) => {
                                match app.focus {
                                    Focus::Right => {
                                        // Set current time for selected field
                                        // Implementation depends on selected field
                                    }
                                    _ => {}
                                }
                            }
                            _ => match app.focus {
                                Focus::Left => handle_left_panel_input(app, key.code),
                                Focus::Right => handle_right_panel_input(app, key.code),
                            },
                        }
                    }
                    _ => match key.code {
                        KeyCode::Enter => {
                            if matches!(app.edit_mode, EditMode::Content) {
                                app.edit_buffer.push('\n');
                            } else {
                                commit_edit(app);
                            }
                        }
                        KeyCode::Esc => {
                            commit_edit(app);
                        }
                        KeyCode::Char(c) => {
                            app.edit_buffer.push(c);
                        }
                        KeyCode::Backspace => {
                            app.edit_buffer.pop();
                        }
                        _ => {}
                    },
                }
            }
            Ok(_) => {} // Ignore other events
            Err(e) => return Err(e),
        }
    }
}

fn handle_left_panel_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => {
            if app.selected_note_idx > 0 {
                app.selected_note_idx -= 1;
                app.list_state.select(Some(app.selected_note_idx));
                app.selected_field_idx = 0;
                app.status_message = get_field_name_at_index(app, app.selected_field_idx);
            }
        }
        KeyCode::Down => {
            if app.selected_note_idx < app.flat_notes.len().saturating_sub(1) {
                app.selected_note_idx += 1;
                app.list_state.select(Some(app.selected_note_idx));
                app.selected_field_idx = 0;
                app.status_message = get_field_name_at_index(app, app.selected_field_idx);
            }
        }
        _ => {}
    }
}

fn handle_right_panel_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => {
            if app.selected_field_idx > 0 {
                app.selected_field_idx -= 1;
                app.status_message = get_field_name_at_index(app, app.selected_field_idx);
            }
        }
        KeyCode::Down => {
            let max_fields = count_visible_fields(app);
            if app.selected_field_idx < max_fields.saturating_sub(1) {
                app.selected_field_idx += 1;
                app.status_message = get_field_name_at_index(app, app.selected_field_idx);
            }
        }
        KeyCode::Enter => {
            start_editing(app);
        }
        _ => {}
    }
}

fn count_visible_fields(app: &App) -> usize {
    let mut count = 0;
    if let Some(note) = app.get_selected_note() {
        if note.status.is_some() {
            count += 1;
        }
        count += 1; // title always visible
        if !note.labels.is_empty() {
            count += 1;
        }
        if let Some(planning) = &note.planning {
            if planning.scheduled.is_some() {
                count += 1;
            }
            if planning.deadline.is_some() {
                count += 1;
            }
            if planning.closed.is_some() {
                count += 1;
            }
        }
        if let Some(logbook) = &note.logbook {
            count += logbook.clock_entries.len();
        }
        count += 1; // content always visible
    }
    count
}

fn get_field_name_at_index(app: &App, field_idx: usize) -> String {
    if let Some(note) = app.get_selected_note() {
        let mut current_idx = 0;

        if let Some(status) = &note.status {
            if current_idx == field_idx {
                return format!("Status: {}", status);
            }
            current_idx += 1;
        }

        if current_idx == field_idx {
            return format!("Title: {}", note.title);
        }
        current_idx += 1;

        if !note.labels.is_empty() {
            if current_idx == field_idx {
                return format!("Labels: :{}:", note.labels.join(":"));
            }
            current_idx += 1;
        }

        if let Some(planning) = &note.planning {
            if let Some(scheduled) = &planning.scheduled {
                if current_idx == field_idx {
                    return format!("Scheduled: {}", scheduled.raw);
                }
                current_idx += 1;
            }
            if let Some(deadline) = &planning.deadline {
                if current_idx == field_idx {
                    return format!("Deadline: {}", deadline.raw);
                }
                current_idx += 1;
            }
            if let Some(closed) = &planning.closed {
                if current_idx == field_idx {
                    return format!("Closed: {}", closed.raw);
                }
                current_idx += 1;
            }
        }

        if let Some(logbook) = &note.logbook {
            for (i, entry) in logbook.clock_entries.iter().enumerate() {
                if current_idx == field_idx {
                    let duration_text = if let Some(duration) = &entry.duration {
                        format!(" => {}", duration)
                    } else {
                        " (running)".to_string()
                    };
                    return format!(
                        "Clock {}: {}{}",
                        i + 1,
                        entry.start.to_datetime_string(),
                        duration_text
                    );
                }
                current_idx += 1;
            }
        }

        if current_idx == field_idx {
            return "Content".to_string();
        }
    }
    "Unknown field".to_string()
}

fn start_editing(app: &mut App) {
    let selected_field_idx = app.selected_field_idx;

    // Clone the data we need to avoid borrowing conflicts
    let (status, title, labels, content, planning, logbook) =
        if let Some(note) = app.get_selected_note() {
            (
                note.status.clone(),
                note.title.clone(),
                note.labels.clone(),
                note.content.clone(),
                note.planning.clone(),
                note.logbook.clone(),
            )
        } else {
            return;
        };

    let mut field_idx = 0;

    if let Some(status_val) = status {
        if field_idx == selected_field_idx {
            app.edit_mode = EditMode::Status;
            app.edit_buffer = status_val;
            app.status_message = "Editing Status - Press Enter to save, Esc to cancel".to_string();
            return;
        }
        field_idx += 1;
    }

    if field_idx == selected_field_idx {
        app.edit_mode = EditMode::Title;
        app.edit_buffer = title;
        app.status_message = "Editing Title - Press Enter to save, Esc to cancel".to_string();
        return;
    }
    field_idx += 1;

    if !labels.is_empty() {
        if field_idx == selected_field_idx {
            app.edit_mode = EditMode::Labels;
            app.edit_buffer = format!(":{}:", labels.join(":"));
            app.status_message = "Editing Labels - Press Enter to save, Esc to cancel".to_string();
            return;
        }
        field_idx += 1;
    }

    // Add planning fields
    if let Some(planning_data) = planning {
        if let Some(scheduled) = &planning_data.scheduled {
            if field_idx == selected_field_idx {
                app.edit_mode = EditMode::Scheduled;
                app.edit_buffer = scheduled.raw.clone();
                app.status_message =
                    "Editing Scheduled - Press Enter to save, Esc to cancel".to_string();
                return;
            }
            field_idx += 1;
        }
        if let Some(deadline) = &planning_data.deadline {
            if field_idx == selected_field_idx {
                app.edit_mode = EditMode::Deadline;
                app.edit_buffer = deadline.raw.clone();
                app.status_message =
                    "Editing Deadline - Press Enter to save, Esc to cancel".to_string();
                return;
            }
            field_idx += 1;
        }
        if let Some(closed) = &planning_data.closed {
            if field_idx == selected_field_idx {
                app.edit_mode = EditMode::Closed;
                app.edit_buffer = closed.raw.clone();
                app.status_message =
                    "Editing Closed - Press Enter to save, Esc to cancel".to_string();
                return;
            }
            field_idx += 1;
        }
    }

    // Add logbook fields (clock entries)
    if let Some(logbook_data) = logbook {
        for (i, entry) in logbook_data.clock_entries.iter().enumerate() {
            if field_idx == selected_field_idx {
                app.edit_mode = EditMode::Content; // Reuse content mode for clock entries
                app.edit_buffer = entry.raw.clone();
                app.status_message = format!(
                    "Editing Clock Entry {} - Press Esc to save, Enter for new line",
                    i + 1
                );
                return;
            }
            field_idx += 1;
        }
    }

    if field_idx == selected_field_idx {
        app.edit_mode = EditMode::Content;
        app.edit_buffer = content;
        app.status_message = "Editing Content - Press Enter to save, Esc to cancel".to_string();
    }
}

fn commit_edit(app: &mut App) {
    let edit_mode = app.edit_mode.clone();
    let edit_buffer = app.edit_buffer.clone();

    // Parse timestamps outside the mutable borrow
    let scheduled_timestamp = if matches!(edit_mode, EditMode::Scheduled) {
        parse_timestamp_from_text(&edit_buffer)
    } else {
        None
    };
    let deadline_timestamp = if matches!(edit_mode, EditMode::Deadline) {
        parse_timestamp_from_text(&edit_buffer)
    } else {
        None
    };
    let closed_timestamp = if matches!(edit_mode, EditMode::Closed) {
        parse_timestamp_from_text(&edit_buffer)
    } else {
        None
    };

    if let Some(note) = app.get_selected_note_mut() {
        match edit_mode {
            EditMode::Status => {
                note.status = if edit_buffer.is_empty() {
                    None
                } else {
                    Some(edit_buffer)
                };
            }
            EditMode::Title => {
                note.title = edit_buffer;
            }
            EditMode::Labels => {
                let labels_str = edit_buffer.trim_start_matches(':').trim_end_matches(':');
                note.labels = if labels_str.is_empty() {
                    Vec::new()
                } else {
                    labels_str.split(':').map(|s| s.to_string()).collect()
                };
            }
            EditMode::Scheduled => {
                if let Some(timestamp) = scheduled_timestamp {
                    if note.planning.is_none() {
                        note.planning = Some(OrgPlanning {
                            scheduled: None,
                            deadline: None,
                            closed: None,
                        });
                    }
                    if let Some(planning) = &mut note.planning {
                        planning.scheduled = Some(timestamp);
                    }
                }
            }
            EditMode::Deadline => {
                if let Some(timestamp) = deadline_timestamp {
                    if note.planning.is_none() {
                        note.planning = Some(OrgPlanning {
                            scheduled: None,
                            deadline: None,
                            closed: None,
                        });
                    }
                    if let Some(planning) = &mut note.planning {
                        planning.deadline = Some(timestamp);
                    }
                }
            }
            EditMode::Closed => {
                if let Some(timestamp) = closed_timestamp {
                    if note.planning.is_none() {
                        note.planning = Some(OrgPlanning {
                            scheduled: None,
                            deadline: None,
                            closed: None,
                        });
                    }
                    if let Some(planning) = &mut note.planning {
                        planning.closed = Some(timestamp);
                    }
                }
            }
            EditMode::Content => {
                note.content = edit_buffer;
            }
            _ => {}
        }

        app.modified = true;
        app.flat_notes = App::flatten_notes(&app.notes);
    }

    app.edit_mode = EditMode::None;
    app.edit_buffer.clear();
    app.status_message = get_field_name_at_index(app, app.selected_field_idx);
}

fn parse_timestamp_from_text(text: &str) -> Option<OrgTimestamp> {
    let parser = OrgParser::new("");
    parser.parse_timestamp_from_text(text)
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(f.size());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    render_left_panel(f, app, main_chunks[0]);
    render_right_panel(f, app, main_chunks[1]);
    render_status_bar(f, app, chunks[1]);
}

fn render_left_panel(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .flat_notes
        .iter()
        .map(|(_, display)| ListItem::new(Line::from(display.clone())))
        .collect();

    let border_style = if matches!(app.focus, Focus::Left) {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Notes")
                .border_style(border_style),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(list, area, &mut app.list_state.clone());
}

fn render_right_panel(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_metadata_panel(f, app, chunks[0]);
    render_content_panel(f, app, chunks[1]);
}

fn render_metadata_panel(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if matches!(app.focus, Focus::Right) {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    if let Some(note) = app.get_selected_note() {
        let mut lines = Vec::new();
        let mut field_idx = 0;

        if let Some(status) = &note.status {
            let style = if field_idx == app.selected_field_idx && matches!(app.focus, Focus::Right)
            {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let text = if matches!(app.edit_mode, EditMode::Status) {
                format!("Status: {}", app.edit_buffer)
            } else {
                format!("Status: {}", status)
            };

            lines.push(Line::from(Span::styled(text, style)));
            field_idx += 1;
        }

        let style = if field_idx == app.selected_field_idx && matches!(app.focus, Focus::Right) {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let text = if matches!(app.edit_mode, EditMode::Title) {
            format!("Title: {}", app.edit_buffer)
        } else {
            format!("Title: {}", note.title)
        };

        lines.push(Line::from(Span::styled(text, style)));
        field_idx += 1;

        if !note.labels.is_empty() {
            let style = if field_idx == app.selected_field_idx && matches!(app.focus, Focus::Right)
            {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let text = if matches!(app.edit_mode, EditMode::Labels) {
                format!("Labels: {}", app.edit_buffer)
            } else {
                format!("Labels: :{}:", note.labels.join(":"))
            };

            lines.push(Line::from(Span::styled(text, style)));
            field_idx += 1;
        }

        if let Some(planning) = &note.planning {
            if let Some(scheduled) = &planning.scheduled {
                let style =
                    if field_idx == app.selected_field_idx && matches!(app.focus, Focus::Right) {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                let text = if matches!(app.edit_mode, EditMode::Scheduled) {
                    format!("Scheduled: {}", app.edit_buffer)
                } else {
                    format!("Scheduled: {}", scheduled.raw)
                };

                lines.push(Line::from(Span::styled(text, style)));
                field_idx += 1;
            }
            if let Some(deadline) = &planning.deadline {
                let style =
                    if field_idx == app.selected_field_idx && matches!(app.focus, Focus::Right) {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                let text = if matches!(app.edit_mode, EditMode::Deadline) {
                    format!("Deadline: {}", app.edit_buffer)
                } else {
                    format!("Deadline: {}", deadline.raw)
                };

                lines.push(Line::from(Span::styled(text, style)));
                field_idx += 1;
            }
            if let Some(closed) = &planning.closed {
                let style =
                    if field_idx == app.selected_field_idx && matches!(app.focus, Focus::Right) {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                let text = if matches!(app.edit_mode, EditMode::Closed) {
                    format!("Closed: {}", app.edit_buffer)
                } else {
                    format!("Closed: {}", closed.raw)
                };

                lines.push(Line::from(Span::styled(text, style)));
                field_idx += 1;
            }
        }

        if let Some(logbook) = &note.logbook {
            if !logbook.clock_entries.is_empty() {
                lines.push(Line::from("Time Tracking:"));
                for entry in &logbook.clock_entries {
                    let style = if field_idx == app.selected_field_idx
                        && matches!(app.focus, Focus::Right)
                    {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let duration_text = if let Some(duration) = &entry.duration {
                        format!(" => {}", duration)
                    } else {
                        " (running)".to_string()
                    };

                    lines.push(Line::from(Span::styled(
                        format!(
                            "  Clock: {}{}",
                            entry.start.to_datetime_string(),
                            duration_text
                        ),
                        style,
                    )));
                    field_idx += 1;
                }

                let total = logbook.format_total_time();
                lines.push(Line::from(format!("  Total: {}", total)));
            }
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Metadata")
                    .border_style(border_style),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let status_text = if app.edit_mode != EditMode::None {
        format!(
            "{}: {}",
            match app.edit_mode {
                EditMode::Status => "STATUS",
                EditMode::Title => "TITLE",
                EditMode::Labels => "LABELS",
                EditMode::Scheduled => "SCHEDULED",
                EditMode::Deadline => "DEADLINE",
                EditMode::Closed => "CLOSED",
                EditMode::Content => "CONTENT",
                EditMode::None => "",
            },
            app.edit_buffer
        )
    } else {
        app.status_message.clone()
    };

    let cursor_style = if app.edit_mode != EditMode::None {
        Style::default().fg(Color::Black).bg(Color::White)
    } else {
        Style::default()
    };

    let paragraph = Paragraph::new(status_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Status")
                .border_style(cursor_style),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);

    // Show cursor when editing non-content fields
    if app.edit_mode != EditMode::None && !matches!(app.edit_mode, EditMode::Content) {
        let prefix_len = match app.edit_mode {
            EditMode::Status => 8,     // "STATUS: ".len()
            EditMode::Title => 7,      // "TITLE: ".len()
            EditMode::Labels => 8,     // "LABELS: ".len()
            EditMode::Scheduled => 11, // "SCHEDULED: ".len()
            EditMode::Deadline => 10,  // "DEADLINE: ".len()
            EditMode::Closed => 8,     // "CLOSED: ".len()
            _ => 0,
        };
        let cursor_x = area.x
            + 1
            + prefix_len
            + (app.edit_buffer.len() as u16).min(area.width.saturating_sub(prefix_len + 3));
        let cursor_y = area.y + 1;
        f.set_cursor(cursor_x, cursor_y);
    }
}

fn render_content_panel(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if matches!(app.focus, Focus::Right) {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    if let Some(note) = app.get_selected_note() {
        let text = if matches!(app.edit_mode, EditMode::Content) {
            app.edit_buffer.clone()
        } else {
            note.content.clone()
        };

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Content")
                    .border_style(border_style),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);

        // Show cursor when editing content
        if matches!(app.edit_mode, EditMode::Content) && matches!(app.focus, Focus::Right) {
            let lines: Vec<&str> = app.edit_buffer.lines().collect();
            let cursor_y = area.y + 1 + (lines.len() as u16).saturating_sub(1);
            let cursor_x = if let Some(last_line) = lines.last() {
                area.x + 1 + (last_line.len() as u16).min(area.width.saturating_sub(3))
            } else {
                area.x + 1
            };
            f.set_cursor(
                cursor_x.min(area.x + area.width - 2),
                cursor_y.min(area.y + area.height - 2),
            );
        }
    }
}

fn main() {
    let matches = Command::new("rorg")
        .version("0.1.0")
        .about("A Rust org-mode file parser")
        .arg(
            Arg::new("file")
                .help("The org-mode file to parse")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Enable verbose output")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .help("Output format (yaml or json)")
                .value_parser(["yaml", "json"])
                .default_value("yaml"),
        )
        .arg(
            Arg::new("summary")
                .short('s')
                .long("summary")
                .help("Show time tracking summary statistics")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("tui")
                .short('t')
                .long("tui")
                .help("Launch TUI interface")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let file_path = matches.get_one::<String>("file").unwrap();
    let verbose = matches.get_flag("verbose");
    let format = matches.get_one::<String>("format").unwrap();
    let show_summary = matches.get_flag("summary");
    let use_tui = matches.get_flag("tui");

    if !Path::new(file_path).exists() {
        eprintln!("Error: File '{}' does not exist", file_path);
        std::process::exit(1);
    }

    let content = match fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(err) => {
            eprintln!("Error reading file '{}': {}", file_path, err);
            std::process::exit(1);
        }
    };

    if verbose {
        println!("Parsing file: {}", file_path);
        println!("File size: {} bytes", content.len());
        println!("Lines: {}", content.lines().count());
        println!();
    }

    let mut parser = OrgParser::new(&content);
    let notes = parser.parse();

    if verbose {
        println!("Found {} top-level notes", notes.len());
        println!();
    }

    if use_tui {
        if let Err(e) = run_tui(notes, file_path.to_string()) {
            eprintln!("Error running TUI: {}", e);
            std::process::exit(1);
        }
    } else {
        if show_summary {
            print_time_summary(&notes);
        }

        match format.as_str() {
            "json" => match serde_json::to_string_pretty(&notes) {
                Ok(json_output) => println!("{}", json_output),
                Err(err) => {
                    eprintln!("Error serializing to JSON: {}", err);
                    std::process::exit(1);
                }
            },
            "yaml" => match serde_yaml::to_string(&notes) {
                Ok(yaml_output) => println!("{}", yaml_output),
                Err(err) => {
                    eprintln!("Error serializing to YAML: {}", err);
                    std::process::exit(1);
                }
            },
            _ => unreachable!(),
        }
    }
}
