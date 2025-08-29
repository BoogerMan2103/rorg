#[cfg(test)]
mod tests {
	use crate::{OrgClockEntry, OrgParser, OrgTimestamp};

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
