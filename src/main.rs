use clap::{Arg, Command};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

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

fn print_notes(notes: &[OrgNote], indent: usize) {
    for note in notes {
        let indent_str = "  ".repeat(indent);

        println!("{}Level: {}", indent_str, note.level);

        if let Some(status) = &note.status {
            println!("{}Status: {}", indent_str, status);
        }

        println!("{}Title: {}", indent_str, note.title);

        if !note.labels.is_empty() {
            println!("{}Labels: {:?}", indent_str, note.labels);
        }

        if let Some(planning) = &note.planning {
            if planning.scheduled.is_some()
                || planning.deadline.is_some()
                || planning.closed.is_some()
            {
                println!("{}Planning:", indent_str);
                if let Some(scheduled) = &planning.scheduled {
                    println!(
                        "{}  Scheduled: {} ({})",
                        indent_str,
                        scheduled.raw,
                        scheduled.to_datetime_string()
                    );
                }
                if let Some(deadline) = &planning.deadline {
                    println!(
                        "{}  Deadline: {} ({})",
                        indent_str,
                        deadline.raw,
                        deadline.to_datetime_string()
                    );
                }
                if let Some(closed) = &planning.closed {
                    println!(
                        "{}  Closed: {} ({})",
                        indent_str,
                        closed.raw,
                        closed.to_datetime_string()
                    );
                }
            }
        }

        if let Some(logbook) = &note.logbook {
            if !logbook.clock_entries.is_empty() {
                println!(
                    "{}Time Tracking: (total: {})",
                    indent_str,
                    logbook.format_total_time()
                );
                for entry in &logbook.clock_entries {
                    if entry.duration.is_some() {
                        println!(
                            "{}  Clock: {} => {}",
                            indent_str,
                            entry.start.to_datetime_string(),
                            entry.format_duration()
                        );
                    } else {
                        println!(
                            "{}  Clock: {} (running)",
                            indent_str,
                            entry.start.to_datetime_string()
                        );
                    }
                }
            }
        }

        if !note.content.trim().is_empty() {
            println!("{}Content:", indent_str);
            for line in note.content.lines() {
                if !line.trim().is_empty() {
                    println!("{}  {}", indent_str, line);
                }
            }
        }

        if !note.children.is_empty() {
            println!("{}Children:", indent_str);
            print_notes(&note.children, indent + 1);
        }

        println!();
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
    println!("=====================");
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
                .help("Output format (text or json)")
                .value_parser(["text", "json"])
                .default_value("text"),
        )
        .arg(
            Arg::new("summary")
                .short('s')
                .long("summary")
                .help("Show time tracking summary statistics")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let file_path = matches.get_one::<String>("file").unwrap();
    let verbose = matches.get_flag("verbose");
    let format = matches.get_one::<String>("format").unwrap();
    let show_summary = matches.get_flag("summary");

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
        "text" => {
            println!("Parsed org-mode structure:");
            println!("========================");
            print_notes(&notes, 0);
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_asterisks() {
        let parser = OrgParser::new("");

        assert_eq!(parser.count_asterisks("* Heading"), Some(1));
        assert_eq!(parser.count_asterisks("** Subheading"), Some(2));
        assert_eq!(parser.count_asterisks("*** Deep heading"), Some(3));
        assert_eq!(parser.count_asterisks("  * Indented heading"), Some(1));
        assert_eq!(parser.count_asterisks("*No space"), None);
        assert_eq!(parser.count_asterisks("Not a heading"), None);
        assert_eq!(parser.count_asterisks(""), None);
    }

    #[test]
    fn test_parse_header_parts_with_status() {
        let parser = OrgParser::new("");

        let (status, title, labels) = parser.parse_header_parts("TODO My task");
        assert_eq!(status, Some("TODO".to_string()));
        assert_eq!(title, "My task");
        assert_eq!(labels, Vec::<String>::new());
    }

    #[test]
    fn test_parse_header_parts_with_tags() {
        let parser = OrgParser::new("");

        let (status, title, labels) = parser.parse_header_parts("TODO My task :urgent:important:");
        assert_eq!(status, Some("TODO".to_string()));
        assert_eq!(title, "My task");
        assert_eq!(labels, vec!["urgent".to_string(), "important".to_string()]);
    }

    #[test]
    fn test_parse_header_parts_no_status() {
        let parser = OrgParser::new("");

        let (status, title, labels) = parser.parse_header_parts("Just a heading :tag:");
        assert_eq!(status, None);
        assert_eq!(title, "Just a heading");
        assert_eq!(labels, vec!["tag".to_string()]);
    }

    #[test]
    fn test_parse_header_parts_no_tags() {
        let parser = OrgParser::new("");

        let (status, title, labels) = parser.parse_header_parts("DONE Completed task");
        assert_eq!(status, Some("DONE".to_string()));
        assert_eq!(title, "Completed task");
        assert_eq!(labels, Vec::<String>::new());
    }

    #[test]
    fn test_parse_simple_org_content() {
        let content = r#"* TODO First task
Some content here.
** DONE Subtask :work:
Subtask content.
* CANCELLED Another task :cancelled:
Final content."#;

        let mut parser = OrgParser::new(content);
        let notes = parser.parse();

        assert_eq!(notes.len(), 2);

        // First note
        assert_eq!(notes[0].level, 1);
        assert_eq!(notes[0].status, Some("TODO".to_string()));
        assert_eq!(notes[0].title, "First task");
        assert_eq!(notes[0].labels, Vec::<String>::new());
        assert_eq!(notes[0].content, "Some content here.");
        assert_eq!(notes[0].children.len(), 1);

        // Child note
        assert_eq!(notes[0].children[0].level, 2);
        assert_eq!(notes[0].children[0].status, Some("DONE".to_string()));
        assert_eq!(notes[0].children[0].title, "Subtask");
        assert_eq!(notes[0].children[0].labels, vec!["work".to_string()]);
        assert_eq!(notes[0].children[0].content, "Subtask content.");

        // Second note
        assert_eq!(notes[1].level, 1);
        assert_eq!(notes[1].status, Some("CANCELLED".to_string()));
        assert_eq!(notes[1].title, "Another task");
        assert_eq!(notes[1].labels, vec!["cancelled".to_string()]);
        assert_eq!(notes[1].content, "Final content.");
    }

    #[test]
    fn test_parse_timestamp() {
        let parser = OrgParser::new("");

        let timestamp = parser
            .parse_timestamp_from_text("[2024-01-01 Mon 10:30]")
            .unwrap();
        assert_eq!(timestamp.year, 2024);
        assert_eq!(timestamp.month, 1);
        assert_eq!(timestamp.day, 1);
        assert_eq!(timestamp.hour, Some(10));
        assert_eq!(timestamp.minute, Some(30));
        assert_eq!(timestamp.day_name, Some("Mon".to_string()));

        let timestamp2 = parser
            .parse_timestamp_from_text("<2023-12-25 Mon>")
            .unwrap();
        assert_eq!(timestamp2.year, 2023);
        assert_eq!(timestamp2.month, 12);
        assert_eq!(timestamp2.day, 25);
        assert_eq!(timestamp2.hour, None);
        assert_eq!(timestamp2.minute, None);
    }

    #[test]
    fn test_parse_planning_keywords() {
        let content = r#"* TODO Task with planning
SCHEDULED: <2024-01-01 Mon 09:00>
DEADLINE: <2024-01-10 Wed>
Some content here."#;

        let mut parser = OrgParser::new(content);
        let notes = parser.parse();

        assert_eq!(notes.len(), 1);
        let note = &notes[0];

        assert!(note.planning.is_some());
        let planning = note.planning.as_ref().unwrap();

        assert!(planning.scheduled.is_some());
        assert_eq!(planning.scheduled.as_ref().unwrap().year, 2024);
        assert_eq!(planning.scheduled.as_ref().unwrap().hour, Some(9));

        assert!(planning.deadline.is_some());
        assert_eq!(planning.deadline.as_ref().unwrap().month, 1);
        assert_eq!(planning.deadline.as_ref().unwrap().day, 10);
    }

    #[test]
    fn test_parse_logbook() {
        let content = r#"* DONE Task with time tracking
:LOGBOOK:
CLOCK: [2024-01-01 Mon 09:00]--[2024-01-01 Mon 12:00] =>  3:00
CLOCK: [2024-01-02 Tue 14:00]--[2024-01-02 Tue 16:30] =>  2:30
:END:
Task completed with time tracking."#;

        let mut parser = OrgParser::new(content);
        let notes = parser.parse();

        assert_eq!(notes.len(), 1);
        let note = &notes[0];

        assert!(note.logbook.is_some());
        let logbook = note.logbook.as_ref().unwrap();

        assert_eq!(logbook.clock_entries.len(), 2);
        assert_eq!(logbook.clock_entries[0].duration, Some("3:00".to_string()));
        assert_eq!(logbook.clock_entries[1].duration, Some("2:30".to_string()));

        // Content should not include logbook
        assert_eq!(note.content, "Task completed with time tracking.");

        // Test total time calculation
        assert_eq!(logbook.total_minutes(), 330); // 3:00 + 2:30 = 5:30 = 330 minutes
        assert_eq!(logbook.format_total_time(), "5h 30m");
    }

    #[test]
    fn test_timestamp_formatting() {
        let timestamp = OrgTimestamp {
            year: 2024,
            month: 1,
            day: 15,
            hour: Some(14),
            minute: Some(30),
            day_name: Some("Mon".to_string()),
            raw: "[2024-01-15 Mon 14:30]".to_string(),
        };

        assert_eq!(timestamp.to_date_string(), "2024-01-15");
        assert_eq!(timestamp.to_datetime_string(), "2024-01-15 14:30");
    }

    #[test]
    fn test_duration_parsing() {
        let clock_entry = OrgClockEntry {
            start: OrgTimestamp {
                year: 2024,
                month: 1,
                day: 1,
                hour: Some(9),
                minute: Some(0),
                day_name: Some("Mon".to_string()),
                raw: "[2024-01-01 Mon 09:00]".to_string(),
            },
            end: None,
            duration: Some("2:30".to_string()),
            raw: "CLOCK: [2024-01-01 Mon 09:00]--[2024-01-01 Mon 11:30] =>  2:30".to_string(),
        };

        assert_eq!(clock_entry.parse_duration_minutes(), Some(150)); // 2:30 = 150 minutes
        assert_eq!(clock_entry.format_duration(), "2:30 (150 minutes)");
    }

    #[test]
    fn test_parse_empty_content() {
        let mut parser = OrgParser::new("");
        let notes = parser.parse();
        assert_eq!(notes.len(), 0);
    }

    #[test]
    fn test_parse_no_headings() {
        let content = "Just some text\nwithout any headings\nat all.";
        let mut parser = OrgParser::new(content);
        let notes = parser.parse();
        assert_eq!(notes.len(), 0);
    }
}
