use clap::{Arg, Command};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgNote {
    pub level: usize,
    pub status: Option<String>,
    pub title: String,
    pub labels: Vec<String>,
    pub content: String,
    pub children: Vec<OrgNote>,
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

        note.content = content_lines.join("\n");
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
        .get_matches();

    let file_path = matches.get_one::<String>("file").unwrap();
    let verbose = matches.get_flag("verbose");
    let format = matches.get_one::<String>("format").unwrap();

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
