use crate::doc::{Doc, PlanKind};
use crate::edit::{NewEntry, Pos, StatusOutcome};
use crate::ts::{self, DateFilter, DateRange};
use chrono::{NaiveDate, NaiveDateTime};
use std::path::Path;

const SAMPLE: &str = "#+TODO: TODO WAIT | DONE CANCELLED
#+FILETAGS: :work:

* TODO [#A] Project Setup                                          :urgent:
SCHEDULED: <2024-01-10 Wed> DEADLINE: <2024-01-12 Fri>
:PROPERTIES:
:ID: setup-1
:END:
:LOGBOOK:
CLOCK: [2024-01-01 Mon 09:00]--[2024-01-01 Mon 10:30] =>  1:30
:END:
Setup body line.
Meeting <2024-01-11 Thu 15:00>.

** DONE Initialize repository :git:
CLOSED: [2024-01-01 Mon 10:00]
Done.

** WAIT Answer from vendor
* Secrets :crypt:
password=hunter2
* Habit
SCHEDULED: <2024-01-01 Mon +1w>
";

fn doc() -> Doc {
	Doc::parse(Path::new("sample.org"), SAMPLE)
}

fn date(s: &str) -> NaiveDate {
	NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn now() -> NaiveDateTime {
	date("2024-02-01").and_hms_opt(12, 30, 0).unwrap()
}

#[test]
fn roundtrip_is_lossless() {
	assert_eq!(doc().render(), SAMPLE);
	let no_nl = "* a\nb";
	assert_eq!(Doc::parse(Path::new("x"), no_nl).render(), no_nl);
}

#[test]
fn parses_structure() {
	let d = doc();
	assert_eq!(d.keywords.todo, vec!["TODO", "WAIT"]);
	assert_eq!(d.headings.len(), 5);
	let h = &d.headings[0];
	assert_eq!(
		(h.keyword.as_deref(), h.priority, h.title.as_str()),
		(Some("TODO"), Some('A'), "Project Setup")
	);
	assert_eq!(h.tags, vec!["urgent"]);
	assert_eq!(
		h.planning.scheduled.as_ref().unwrap().date,
		date("2024-01-10")
	);
	assert_eq!(
		h.planning.deadline.as_ref().unwrap().date,
		date("2024-01-12")
	);
	assert_eq!(h.prop("id"), Some("setup-1"));
	assert_eq!(h.clock_minutes(), 90);
	assert_eq!(
		&d.lines[d.body_range(0)],
		&["Setup body line.", "Meeting <2024-01-11 Thu 15:00>."]
	);
	assert!(
		h.timestamps
			.iter()
			.any(|t| t.active && t.date == date("2024-01-11"))
	);
	assert_eq!(d.headings[1].id(), "1.1");
	assert_eq!(d.headings[2].id(), "1.2");
	assert_eq!(d.all_tags(1), vec!["work", "urgent", "git"]);
}

#[test]
fn keyword_must_be_known() {
	let d = Doc::parse(
		Path::new("x"),
		"#+TODO: TODO | DONE\n* WAIT not a keyword here\n",
	);
	assert_eq!(d.headings[0].keyword, None);
	assert_eq!(d.headings[0].title, "WAIT not a keyword here");
}

#[test]
fn selectors() {
	let d = doc();
	assert_eq!(d.resolve("1.2").unwrap(), 2);
	assert_eq!(d.resolve("L12").unwrap(), 0);
	assert_eq!(d.resolve("id:setup-1").unwrap(), 0);
	assert_eq!(d.resolve("@vendor").unwrap(), 2);
	assert_eq!(d.resolve("@=Habit").unwrap(), 4);
	assert!(d.resolve("@e").unwrap_err().contains("ambiguous"));
	assert!(d.resolve("9.9").is_err());
	assert!(
		d.resolve("L1")
			.unwrap_err()
			.contains("before the first heading")
	);
	// pasted heading line, with or without @
	assert_eq!(d.resolve("@** WAIT Answer from vendor").unwrap(), 2);
	assert_eq!(
		d.resolve("* TODO [#A] Project Setup   :urgent:").unwrap(),
		0
	);
	assert_eq!(d.resolve("answer from").unwrap(), 2);
	assert!(d.resolve("nothing like this").is_err());
}

#[test]
fn protected_tags() {
	let d = doc();
	assert!(d.is_protected(3));
	assert!(!d.is_protected(0));
}

#[test]
fn date_ranges_and_repeaters() {
	let today = date("2024-01-10");
	assert_eq!(
		ts::parse_date_expr("tomorrow", today).unwrap(),
		date("2024-01-11")
	);
	assert_eq!(
		ts::parse_date_expr("+1w", today).unwrap(),
		date("2024-01-17")
	);
	assert_eq!(
		ts::parse_date_expr("fri", today).unwrap(),
		date("2024-01-12")
	);
	assert_eq!(ts::parse_date_expr("wed", today).unwrap(), today);
	let r = DateRange::parse("..yesterday", today).unwrap();
	assert_eq!((r.start, r.end), (None, Some(date("2024-01-09"))));

	let d = doc();
	let habit = d.headings[4].planning.scheduled.clone().unwrap();
	assert!(habit.occurs_in(&DateRange::parse("2024-01-15", today).unwrap()));
	assert!(!habit.occurs_in(&DateRange::parse("2024-01-16", today).unwrap()));
	assert!(DateFilter::parse("none", today).unwrap().matches(None));
	assert_eq!(
		ts::parse_when("2024-03-01 10:00 +1w", today, false).unwrap(),
		"<2024-03-01 Fri 10:00 +1w>"
	);
	assert_eq!(
		ts::parse_when("2024-03-01", today, true).unwrap(),
		"<2024-03-01 Пт>"
	);
	assert!(ts::parse_when("2024-03-01 junk", today, false).is_err());
	assert!(!ts::detect_ru(
		"<2024-01-01 Mon> <2024-01-02 Tue> <2024-01-03 Ср>"
	));
	assert!(ts::detect_ru(
		"<2024-01-01 Пн> <2024-01-02 Вт> <2024-01-03 Wed>"
	));
}

#[test]
fn status_done_adds_and_removes_closed() {
	let mut d = doc();
	d.set_keyword(2, Some("DONE".into()), true, now()).unwrap();
	assert_eq!(d.lines[18], "** DONE Answer from vendor");
	assert_eq!(d.lines[19], "CLOSED: [2024-02-01 Thu 12:30]");
	d.set_keyword(2, Some("TODO".into()), true, now()).unwrap();
	assert_eq!(
		d.render(),
		SAMPLE.replace("** WAIT Answer", "** TODO Answer")
	);
	assert!(d.set_keyword(2, Some("NOPE".into()), true, now()).is_err());
}

#[test]
fn status_keeps_tag_alignment() {
	let mut d = doc();
	let before = d.lines[3].chars().count();
	d.set_keyword(0, Some("WAIT".into()), false, now()).unwrap();
	assert!(d.lines[3].starts_with("* WAIT [#A] Project Setup  "));
	assert_eq!(d.lines[3].chars().count(), before);
}

#[test]
fn repeating_task_shifts_instead_of_closing() {
	let mut d = doc();
	let i = d.resolve("@habit").unwrap();
	d.headings[i].keyword = None;
	assert!(matches!(
		d.set_keyword(i, Some("DONE".into()), true, now()).unwrap(),
		StatusOutcome::Repeated
	));
	let h = &d.headings[i];
	assert_eq!(
		h.planning.scheduled.as_ref().unwrap().raw,
		"<2024-01-08 Mon +1w>"
	);
	assert!(h.planning.closed.is_none());
	assert!(h.prop("LAST_REPEAT").is_some());
}

#[test]
fn planning_edits() {
	let mut d = doc();
	d.set_planning(0, PlanKind::Scheduled, None).unwrap();
	assert_eq!(d.lines[4], "DEADLINE: <2024-01-12 Fri>");
	d.set_planning(0, PlanKind::Deadline, None).unwrap();
	assert_eq!(d.lines[4], ":PROPERTIES:");
	d.set_planning(2, PlanKind::Scheduled, Some("<2024-02-02 Fri>".into()))
		.unwrap();
	let i = d.resolve("@vendor").unwrap();
	assert_eq!(
		d.lines[d.headings[i].line + 1],
		"SCHEDULED: <2024-02-02 Fri>"
	);
	d.set_planning(i, PlanKind::Deadline, Some("<2024-02-05 Mon>".into()))
		.unwrap();
	assert_eq!(
		d.lines[d.headings[i].line + 1],
		"SCHEDULED: <2024-02-02 Fri> DEADLINE: <2024-02-05 Mon>"
	);
}

#[test]
fn properties() {
	let mut d = doc();
	d.set_property(0, "OWNER", Some("me")).unwrap();
	assert_eq!(d.headings[0].prop("OWNER"), Some("me"));
	d.set_property(0, "OWNER", None).unwrap();
	d.set_property(0, "ID", None).unwrap();
	assert!(d.headings[0].props_range.is_none());
	d.set_property(2, "X", Some("1")).unwrap();
	let l = d.headings[2].line;
	assert_eq!(&d.lines[l + 1..l + 4], &[":PROPERTIES:", ":X: 1", ":END:"]);
}

#[test]
fn body_edits() {
	let mut d = doc();
	d.append_body(0, "Appended.").unwrap();
	assert_eq!(&d.lines[d.body_range(0)].last().unwrap()[..], "Appended.");
	assert_eq!(
		d.lines[d.body_range(0).end],
		"",
		"blank separator before child kept"
	);
	d.set_body(0, "Only this.").unwrap();
	assert_eq!(&d.lines[d.body_range(0)], &["Only this."]);
	assert!(
		d.lines.iter().any(|l| l.starts_with("CLOCK:")),
		"logbook untouched"
	);
	assert!(d.append_body(0, "* sneaky heading").is_err());
}

#[test]
fn add_move_delete() {
	let mut d = doc();
	let entry = |title| NewEntry {
		title,
		keyword: Some("TODO".into()),
		priority: None,
		tags: vec!["x".into()],
		scheduled: Some("<2024-02-02 Fri>".into()),
		deadline: None,
		body: Some("note"),
	};
	let i = d
		.add_heading(
			Pos::Under {
				parent: 0,
				first: false,
			},
			entry("New child"),
		)
		.unwrap();
	assert_eq!(d.headings[i].id(), "1.3");
	assert_eq!(d.lines[d.headings[i].line], "** TODO New child :x:");
	let j = d
		.add_heading(
			Pos::Under {
				parent: 0,
				first: true,
			},
			entry("First child"),
		)
		.unwrap();
	assert_eq!(d.headings[j].id(), "1.1");

	let k = d.resolve("@=New child").unwrap();
	let k = d.move_subtree(k, Pos::Top { first: false }).unwrap();
	assert_eq!(d.lines[d.headings[k].line], "* TODO New child :x:");
	assert_eq!(d.headings[k].id(), "4");

	let k = d
		.move_subtree(
			k,
			Pos::Under {
				parent: d.resolve("@vendor").unwrap(),
				first: false,
			},
		)
		.unwrap();
	assert_eq!(d.lines[d.headings[k].line], "*** TODO New child :x:");
	assert!(
		d.move_subtree(
			0,
			Pos::Under {
				parent: k,
				first: false
			}
		)
		.is_err()
	);

	let n = d.resolve("@=First child").unwrap();
	d.delete_subtree(n);
	let k = d.resolve("@=New child").unwrap();
	d.delete_subtree(k);
	assert_eq!(d.render(), SAMPLE);
}

#[test]
fn notes() {
	let mut d = doc();
	let v = d.resolve("@vendor").unwrap();
	d.add_note(v, "first\nsecond", now(), false).unwrap();
	d.add_note(v, "newer", now(), false).unwrap();
	let l = d.headings[v].line;
	assert_eq!(
		&d.lines[l + 1..l + 6],
		&[
			"- Note taken on [2024-02-01 Thu 12:30] \\\\",
			"  newer",
			"- Note taken on [2024-02-01 Thu 12:30] \\\\",
			"  first",
			"  second",
		]
	);

	// into existing LOGBOOK, at its top, before CLOCK lines
	d.add_note(0, "logged", now(), true).unwrap();
	let lb = d.lines.iter().position(|l| l == ":LOGBOOK:").unwrap();
	assert_eq!(
		d.lines[lb + 1],
		"- Note taken on [2024-02-01 Thu 12:30] \\\\"
	);
	assert_eq!(d.lines[lb + 2], "  logged");
	assert!(d.lines[lb + 3].starts_with("CLOCK:"));

	// new LOGBOOK created after planning when missing
	let h = d.resolve("@habit").unwrap();
	d.add_note(h, "x", now(), true).unwrap();
	let l = d.headings[h].line;
	assert_eq!(
		&d.lines[l + 2..l + 6],
		&[
			":LOGBOOK:",
			"- Note taken on [2024-02-01 Thu 12:30] \\\\",
			"  x",
			":END:"
		]
	);
	assert!(d.add_note(h, "  \n", now(), false).is_err());
}
