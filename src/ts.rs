//! Org timestamps, date expressions and date ranges.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, Weekday};
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

static TS_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"([<\[])(\d{4}-\d{2}-\d{2})([^<>\[\]\n]*)([>\]])").unwrap());
static REPEAT_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^(\+\+|\.\+|\+)(\d+)([hdwmy])$").unwrap());
static WARN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^--?\d+[hdwmy]$").unwrap());
static TIME_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^\d{1,2}:\d{2}(-\d{1,2}:\d{2})?$").unwrap());
static REL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([+-])(\d+)([dwmy])$").unwrap());

const DAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const DAYS_RU: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

#[derive(Debug, Clone, Serialize)]
pub struct Repeater {
	/// "+", "++" or ".+"
	pub kind: String,
	pub n: u32,
	pub unit: char,
}

impl Repeater {
	pub fn step(&self, d: NaiveDate) -> Option<NaiveDate> {
		match self.unit {
			'd' => d.checked_add_signed(Duration::days(self.n as i64)),
			'w' => d.checked_add_signed(Duration::weeks(self.n as i64)),
			'm' => d.checked_add_months(Months::new(self.n)),
			'y' => d.checked_add_months(Months::new(self.n * 12)),
			_ => None,
		}
	}

	/// Next base date after marking a repeating task done (org semantics).
	pub fn bump(&self, d: NaiveDate, today: NaiveDate) -> Option<NaiveDate> {
		match self.kind.as_str() {
			"++" => {
				let mut next = self.step(d)?;
				while next <= today {
					next = self.step(next)?;
				}
				Some(next)
			},
			".+" => self.step(today),
			_ => self.step(d),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct Timestamp {
	pub date: NaiveDate,
	pub time: Option<String>,
	pub active: bool,
	pub repeater: Option<Repeater>,
	pub raw: String,
}

impl Timestamp {
	/// Compact form for listings: `2024-01-01 10:00 +1w`.
	pub fn short(&self) -> String {
		let mut s = self.date.format("%Y-%m-%d").to_string();
		if let Some(t) = &self.time {
			s.push(' ');
			s.push_str(t);
		}
		if let Some(r) = &self.repeater {
			s.push_str(&format!(" {}{}{}", r.kind, r.n, r.unit));
		}
		s
	}

	/// Dates in `r` on which this timestamp occurs (expanding repeaters), at most `limit`.
	pub fn occurrences(&self, r: &DateRange, limit: usize) -> Vec<NaiveDate> {
		let mut out = Vec::new();
		let mut d = self.date;
		for _ in 0..5000 {
			if r.end.is_some_and(|e| d > e) || out.len() >= limit {
				break;
			}
			if r.contains(d) {
				out.push(d);
			}
			match self.repeater.as_ref().and_then(|rep| rep.step(d)) {
				Some(next) => d = next,
				None => break,
			}
		}
		out
	}

	pub fn occurs_in(&self, r: &DateRange) -> bool {
		!self.occurrences(r, 1).is_empty()
	}
}

/// All timestamps found in `text`.
pub fn find_all(text: &str) -> Vec<Timestamp> {
	TS_RE
		.captures_iter(text)
		.filter_map(|c| {
			let active = &c[1] == "<";
			if active != (&c[4] == ">") {
				return None;
			}
			let date = NaiveDate::parse_from_str(&c[2], "%Y-%m-%d").ok()?;
			let mut time = None;
			let mut repeater = None;
			for tok in c[3].split_whitespace() {
				if TIME_RE.is_match(tok) {
					time = Some(tok.to_string());
				} else if let Some(r) = REPEAT_RE.captures(tok) {
					let n: u32 = r[2].parse().ok()?;
					if n > 0 && &r[3] != "h" {
						repeater = Some(Repeater {
							kind: r[1].to_string(),
							n,
							unit: r[3].chars().next().unwrap(),
						});
					}
				}
			}
			Some(Timestamp {
				date,
				time,
				active,
				repeater,
				raw: c[0].to_string(),
			})
		})
		.collect()
}

pub fn day_name(d: NaiveDate, ru: bool) -> &'static str {
	let i = d.weekday().num_days_from_monday() as usize;
	if ru { DAYS_RU[i] } else { DAYS_EN[i] }
}

fn is_cyrillic(s: &str) -> bool {
	s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

/// True if most timestamps with a day name use Russian day names.
pub fn detect_ru(text: &str) -> bool {
	let (mut ru, mut other) = (0, 0);
	for c in TS_RE.captures_iter(text) {
		match c[3]
			.split_whitespace()
			.find(|t| t.chars().all(char::is_alphabetic))
		{
			Some(day) if is_cyrillic(day) => ru += 1,
			Some(_) => other += 1,
			None => {},
		}
	}
	ru > other
}

pub fn format(date: NaiveDate, extras: &[String], active: bool, ru: bool) -> String {
	let (o, c) = if active { ('<', '>') } else { ('[', ']') };
	let mut s = format!("{o}{} {}", date.format("%Y-%m-%d"), day_name(date, ru));
	for e in extras {
		s.push(' ');
		s.push_str(e);
	}
	s.push(c);
	s
}

pub fn format_now(now: NaiveDateTime, ru: bool) -> String {
	format(now.date(), &[now.format("%H:%M").to_string()], false, ru)
}

/// Same timestamp moved to another date, keeping time, repeater etc.
pub fn with_date(raw: &str, date: NaiveDate, ru_default: bool) -> String {
	let Some(c) = TS_RE.captures(raw) else {
		return raw.to_string();
	};
	let ru = if is_cyrillic(&c[3]) { true } else { ru_default };
	let extras: Vec<String> = c[3]
		.split_whitespace()
		.filter(|t| !t.chars().all(char::is_alphabetic))
		.map(str::to_string)
		.collect();
	format(date, &extras, &c[1] == "<", ru)
}

fn parse_weekday(s: &str) -> Option<Weekday> {
	let s = s.to_lowercase();
	let table: [(&[&str], Weekday); 7] = [
		(&["mon", "monday", "пн", "понедельник"], Weekday::Mon),
		(&["tue", "tuesday", "вт", "вторник"], Weekday::Tue),
		(&["wed", "wednesday", "ср", "среда"], Weekday::Wed),
		(&["thu", "thursday", "чт", "четверг"], Weekday::Thu),
		(&["fri", "friday", "пт", "пятница"], Weekday::Fri),
		(&["sat", "saturday", "сб", "суббота"], Weekday::Sat),
		(&["sun", "sunday", "вс", "воскресенье"], Weekday::Sun),
	];
	table
		.iter()
		.find(|(names, _)| names.contains(&s.as_str()))
		.map(|(_, w)| *w)
}

/// `2024-01-31`, `today`, `tomorrow`, `yesterday`, `+3d`, `-1w`, `+2m`, `fri` (next friday, today included).
pub fn parse_date_expr(s: &str, today: NaiveDate) -> Result<NaiveDate, String> {
	let l = s.trim().to_lowercase();
	match l.as_str() {
		"today" | "сегодня" => return Ok(today),
		"tomorrow" | "завтра" => return Ok(today + Duration::days(1)),
		"yesterday" | "вчера" => return Ok(today - Duration::days(1)),
		_ => {},
	}
	if let Ok(d) = NaiveDate::parse_from_str(&l, "%Y-%m-%d") {
		return Ok(d);
	}
	if let Some(c) = REL_RE.captures(&l) {
		let n: u32 = c[2].parse().map_err(|_| format!("bad number in '{s}'"))?;
		let fwd = &c[1] == "+";
		let d = match &c[3] {
			"d" | "w" => {
				let days = if &c[3] == "w" { n as i64 * 7 } else { n as i64 };
				Some(if fwd {
					today + Duration::days(days)
				} else {
					today - Duration::days(days)
				})
			},
			unit => {
				let m = Months::new(if unit == "y" { n * 12 } else { n });
				if fwd {
					today.checked_add_months(m)
				} else {
					today.checked_sub_months(m)
				}
			},
		};
		return d.ok_or_else(|| format!("date out of range: '{s}'"));
	}
	if let Some(w) = parse_weekday(&l) {
		let diff = (7 + w.num_days_from_monday() as i64
			- today.weekday().num_days_from_monday() as i64)
			% 7;
		return Ok(today + Duration::days(diff));
	}
	Err(format!(
		"cannot parse date '{s}' (use YYYY-MM-DD, today, tomorrow, yesterday, +3d, -1w, +1m, fri)"
	))
}

/// User input for SCHEDULED/DEADLINE: a raw org timestamp, or `DATE [HH:MM[-HH:MM]] [+1w] [-2d]`.
pub fn parse_when(s: &str, today: NaiveDate, ru: bool) -> Result<String, String> {
	let s = s.trim();
	if s.starts_with('<') || s.starts_with('[') {
		let all = find_all(s);
		if all.len() == 1 && all[0].raw == s {
			return Ok(s.to_string());
		}
		return Err(format!("not a valid org timestamp: '{s}'"));
	}
	let mut toks = s.split_whitespace();
	let date = parse_date_expr(toks.next().ok_or("empty date")?, today)?;
	let mut extras = Vec::new();
	for t in toks {
		if TIME_RE.is_match(t) || REPEAT_RE.is_match(t) || WARN_RE.is_match(t) {
			extras.push(t.to_string());
		} else {
			return Err(format!(
				"unexpected '{t}' in '{s}' (expected HH:MM, +1w repeater or -2d warning)"
			));
		}
	}
	Ok(format(date, &extras, true, ru))
}

#[derive(Debug, Clone, Default)]
pub struct DateRange {
	pub start: Option<NaiveDate>,
	pub end: Option<NaiveDate>,
}

impl DateRange {
	/// `DATE`, `A..B`, `..B`, `A..`, or shortcuts `overdue` (..yesterday), `week` (today..+6d).
	pub fn parse(s: &str, today: NaiveDate) -> Result<Self, String> {
		match s.trim() {
			"overdue" => {
				return Ok(Self {
					start: None,
					end: Some(today - Duration::days(1)),
				});
			},
			"week" => {
				return Ok(Self {
					start: Some(today),
					end: Some(today + Duration::days(6)),
				});
			},
			_ => {},
		}
		let side = |x: &str| -> Result<Option<NaiveDate>, String> {
			if x.trim().is_empty() {
				Ok(None)
			} else {
				parse_date_expr(x, today).map(Some)
			}
		};
		if let Some((a, b)) = s.split_once("..") {
			Ok(Self {
				start: side(a)?,
				end: side(b)?,
			})
		} else {
			let d = parse_date_expr(s, today)?;
			Ok(Self {
				start: Some(d),
				end: Some(d),
			})
		}
	}

	pub fn contains(&self, d: NaiveDate) -> bool {
		self.start.is_none_or(|s| d >= s) && self.end.is_none_or(|e| d <= e)
	}

	pub fn describe(&self) -> String {
		let f = |d: Option<NaiveDate>| {
			d.map(|d| d.format("%Y-%m-%d").to_string())
				.unwrap_or_default()
		};
		if self.start.is_some() && self.start == self.end {
			f(self.start)
		} else {
			format!("{}..{}", f(self.start), f(self.end))
		}
	}
}

/// Filter on a planning date: `any`, `none`, or a range expression.
#[derive(Debug, Clone)]
pub enum DateFilter {
	Any,
	None,
	Range(DateRange),
}

impl DateFilter {
	pub fn parse(s: &str, today: NaiveDate) -> Result<Self, String> {
		match s.trim() {
			"any" => Ok(Self::Any),
			"none" => Ok(Self::None),
			other => DateRange::parse(other, today).map(Self::Range),
		}
	}

	pub fn matches(&self, ts: Option<&Timestamp>) -> bool {
		match (self, ts) {
			(Self::Any, t) => t.is_some(),
			(Self::None, t) => t.is_none(),
			(Self::Range(r), Some(t)) => t.occurs_in(r),
			(Self::Range(_), None) => false,
		}
	}
}
