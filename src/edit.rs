//! Mutations. Every edit is a line splice recorded in `Doc::changes`, followed by a reparse.

use crate::doc::{Change, Doc, PlanKind, build_heading_line, is_heading_line, prop_re};
use crate::ts;
use chrono::NaiveDateTime;
use regex::Regex;

/// Where to put a new or moved heading.
#[derive(Debug, Clone, Copy)]
pub enum Pos {
	Under { parent: usize, first: bool },
	After(usize),
	Before(usize),
	Top { first: bool },
}

pub struct NewEntry<'a> {
	pub title: &'a str,
	pub keyword: Option<String>,
	pub priority: Option<char>,
	pub tags: Vec<String>,
	pub scheduled: Option<String>,
	pub deadline: Option<String>,
	pub body: Option<&'a str>,
}

pub enum StatusOutcome {
	Set,
	/// Repeating task: dates were shifted instead of closing it.
	Repeated,
}

fn leading_ws(s: &str) -> &str {
	&s[..s.len() - s.trim_start().len()]
}

impl Doc {
	fn splice(&mut self, start: usize, remove: usize, add: Vec<String>, quiet: bool) {
		let removed: Vec<String> = self
			.lines
			.splice(start..start + remove, add.clone())
			.collect();
		self.changes.push(Change {
			line: start,
			removed,
			added: add,
			quiet,
		});
		self.reparse();
	}

	fn replace_line(&mut self, j: usize, new: String) {
		if self.lines[j] != new {
			self.splice(j, 1, vec![new], false);
		}
	}

	/// Indentation for new planning/drawer lines, following the file's existing style.
	fn meta_indent(&self, i: usize) -> String {
		let h = &self.headings[i];
		if h.planning_end > h.line + 1 {
			return leading_ws(&self.lines[h.line + 1]).to_string();
		}
		if let Some(r) = &h.props_range {
			return leading_ws(&self.lines[r.start]).to_string();
		}
		let indented = self.headings.iter().any(|o| {
			o.planning_end > o.line + 1 && !leading_ws(&self.lines[o.line + 1]).is_empty()
		});
		if indented {
			" ".repeat(h.level + 1)
		} else {
			String::new()
		}
	}

	fn rewrite_heading(
		&mut self,
		i: usize,
		keyword: Option<&str>,
		priority: Option<char>,
		title: &str,
		tags: &[String],
	) {
		let h = &self.headings[i];
		let line = build_heading_line(h.level, keyword, priority, title, tags, h.tags_align_end);
		self.replace_line(h.line, line);
	}

	pub fn set_title(&mut self, i: usize, title: &str) -> Result<(), String> {
		let title = title.trim();
		if title.contains('\n') {
			return Err("title must be a single line".into());
		}
		let h = self.headings[i].clone();
		self.rewrite_heading(i, h.keyword.as_deref(), h.priority, title, &h.tags);
		Ok(())
	}

	pub fn set_tags(&mut self, i: usize, tags: Vec<String>) -> Result<(), String> {
		if let Some(bad) = tags
			.iter()
			.find(|t| t.is_empty() || !t.chars().all(|c| c.is_alphanumeric() || "_@#%".contains(c)))
		{
			return Err(format!(
				"invalid tag '{bad}' (allowed: letters, digits, _ @ # %)"
			));
		}
		let h = self.headings[i].clone();
		self.rewrite_heading(i, h.keyword.as_deref(), h.priority, &h.title, &tags);
		Ok(())
	}

	pub fn set_priority(&mut self, i: usize, p: Option<char>) -> Result<(), String> {
		let h = self.headings[i].clone();
		self.rewrite_heading(i, h.keyword.as_deref(), p, &h.title, &h.tags);
		Ok(())
	}

	pub fn set_keyword(
		&mut self,
		i: usize,
		new: Option<String>,
		log: bool,
		now: NaiveDateTime,
	) -> Result<StatusOutcome, String> {
		if let Some(k) = &new
			&& !self.keywords.contains(k)
		{
			return Err(format!(
				"unknown keyword '{k}'; known: {}",
				self.keywords.describe()
			));
		}
		let h = self.headings[i].clone();
		let was_done = h
			.keyword
			.as_deref()
			.is_some_and(|k| self.keywords.is_done(k));
		let will_done = new.as_deref().is_some_and(|k| self.keywords.is_done(k));
		let repeating: Vec<(PlanKind, String)> = [PlanKind::Scheduled, PlanKind::Deadline]
			.into_iter()
			.filter_map(|k| {
				let t = h.planning.get(k)?;
				let rep = t.repeater.as_ref()?;
				let date = rep.bump(t.date, now.date())?;
				Some((k, ts::with_date(&t.raw, date, self.ru_days)))
			})
			.collect();

		if will_done && !was_done && !repeating.is_empty() {
			for (k, raw) in repeating {
				self.set_planning(i, k, Some(raw))?;
			}
			if log {
				self.set_property(i, "LAST_REPEAT", Some(&ts::format_now(now, self.ru_days)))?;
			}
			let keep = h
				.keyword
				.clone()
				.or_else(|| self.keywords.todo.first().cloned());
			let h = self.headings[i].clone();
			self.rewrite_heading(i, keep.as_deref(), h.priority, &h.title, &h.tags);
			return Ok(StatusOutcome::Repeated);
		}

		self.rewrite_heading(i, new.as_deref(), h.priority, &h.title, &h.tags);
		if log && will_done && !was_done {
			self.set_planning(i, PlanKind::Closed, Some(ts::format_now(now, self.ru_days)))?;
		} else if log && !will_done && h.planning.closed.is_some() {
			self.set_planning(i, PlanKind::Closed, None)?;
		}
		Ok(StatusOutcome::Set)
	}

	/// Set (or remove with `None`) SCHEDULED/DEADLINE/CLOSED. `value` is a full org timestamp.
	pub fn set_planning(
		&mut self,
		i: usize,
		kind: PlanKind,
		value: Option<String>,
	) -> Result<(), String> {
		let kw = kind.keyword();
		let item = Regex::new(&format!(r"\s*{kw}:\s*[<\[][^<>\[\]\n]*[>\]]")).unwrap();
		let h = self.headings[i].clone();
		for j in h.line + 1..h.planning_end {
			let line = self.lines[j].clone();
			let Some(m) = item.find(&line) else { continue };
			let indent = leading_ws(&line);
			let rest = format!("{}{}", &line[..m.start()], &line[m.end()..]);
			match value {
				Some(v) => {
					let sep = if m.start() == 0 || rest[..m.start()].trim().is_empty() {
						""
					} else {
						" "
					};
					let new = format!("{}{sep}{kw}: {v}{}", &line[..m.start()], &line[m.end()..]);
					let new = format!("{indent}{}", new.trim_start());
					self.replace_line(j, new);
				},
				None if rest.trim().is_empty() => self.splice(j, 1, vec![], false),
				None => self.replace_line(j, format!("{indent}{}", rest.trim())),
			}
			return Ok(());
		}
		let Some(v) = value else { return Ok(()) };
		if h.planning_end > h.line + 1 {
			let j = h.line + 1;
			let new = format!("{} {kw}: {v}", self.lines[j].trim_end());
			self.replace_line(j, new);
		} else {
			let indent = self.meta_indent(i);
			self.splice(h.line + 1, 0, vec![format!("{indent}{kw}: {v}")], false);
		}
		Ok(())
	}

	pub fn set_property(&mut self, i: usize, key: &str, value: Option<&str>) -> Result<(), String> {
		if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c == ':') {
			return Err(format!("invalid property name '{key}'"));
		}
		let h = self.headings[i].clone();
		let indent = self.meta_indent(i);
		let fmt = |v: &str| format!("{indent}:{key}: {v}").trim_end().to_string();
		if let Some(r) = &h.props_range {
			for j in r.start + 1..r.end - 1 {
				let Some(c) = prop_re().captures(&self.lines[j]) else {
					continue;
				};
				if !c[1].eq_ignore_ascii_case(key) {
					continue;
				}
				match value {
					Some(v) => self.replace_line(j, fmt(v)),
					None if r.end - r.start == 3 => self.splice(r.start, 3, vec![], false),
					None => self.splice(j, 1, vec![], false),
				}
				return Ok(());
			}
			if let Some(v) = value {
				self.splice(r.end - 1, 0, vec![fmt(v)], false);
			}
		} else if let Some(v) = value {
			let lines = vec![
				format!("{indent}:PROPERTIES:"),
				fmt(v),
				format!("{indent}:END:"),
			];
			self.splice(h.planning_end, 0, lines, false);
		}
		Ok(())
	}

	fn body_lines(text: &str) -> Result<Vec<String>, String> {
		let lines: Vec<String> = text
			.trim_end_matches('\n')
			.split('\n')
			.map(str::to_string)
			.collect();
		if let Some((n, l)) = lines.iter().enumerate().find(|(_, l)| is_heading_line(l)) {
			return Err(format!(
				"body line {} looks like a heading ('{}'); use `add` to create headings",
				n + 1,
				l.chars().take(40).collect::<String>()
			));
		}
		Ok(lines)
	}

	pub fn append_body(&mut self, i: usize, text: &str) -> Result<(), String> {
		let lines = Self::body_lines(text)?;
		let r = self.body_range(i);
		self.splice(r.end, 0, lines, false);
		Ok(())
	}

	pub fn set_body(&mut self, i: usize, text: &str) -> Result<(), String> {
		let lines = if text.trim().is_empty() {
			vec![]
		} else {
			Self::body_lines(text)?
		};
		let r = self.body_range(i);
		self.splice(r.start, r.end - r.start, lines, false);
		Ok(())
	}

	/// Line index and level at which a heading should be inserted.
	fn insert_point(&self, pos: Pos) -> (usize, usize) {
		match pos {
			Pos::Under { parent, first } => {
				let p = &self.headings[parent];
				if first && !p.children.is_empty() {
					(p.section_end, p.level + 1)
				} else {
					(self.after_last_nonblank(p.line, p.subtree_end), p.level + 1)
				}
			},
			Pos::After(s) => {
				let h = &self.headings[s];
				(self.after_last_nonblank(h.line, h.subtree_end), h.level)
			},
			Pos::Before(s) => (self.headings[s].line, self.headings[s].level),
			Pos::Top { first: true } => (
				self.headings.first().map_or(self.lines.len(), |h| h.line),
				1,
			),
			Pos::Top { first: false } => (self.after_last_nonblank(0, self.lines.len()), 1),
		}
	}

	/// Returns the index of the new heading.
	pub fn add_heading(&mut self, pos: Pos, e: NewEntry) -> Result<usize, String> {
		if e.title.trim().is_empty() || e.title.contains('\n') {
			return Err("title must be a non-empty single line".into());
		}
		if let Some(k) = &e.keyword
			&& !self.keywords.contains(k)
		{
			return Err(format!(
				"unknown keyword '{k}'; known: {}",
				self.keywords.describe()
			));
		}
		let (at, level) = self.insert_point(pos);
		let mut lines = vec![build_heading_line(
			level,
			e.keyword.as_deref(),
			e.priority,
			e.title.trim(),
			&e.tags,
			None,
		)];
		let planning: Vec<String> = [("DEADLINE", &e.deadline), ("SCHEDULED", &e.scheduled)]
			.into_iter()
			.filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}: {v}")))
			.collect();
		if !planning.is_empty() {
			let indented = self.headings.iter().any(|o| {
				o.planning_end > o.line + 1 && !leading_ws(&self.lines[o.line + 1]).is_empty()
			});
			let indent = if indented {
				" ".repeat(level + 1)
			} else {
				String::new()
			};
			lines.push(format!("{indent}{}", planning.join(" ")));
		}
		if let Some(b) = e.body.filter(|b| !b.trim().is_empty()) {
			lines.extend(Self::body_lines(b)?);
		}
		self.splice(at, 0, lines, false);
		self.heading_at_line(at)
			.ok_or_else(|| "internal: inserted heading not found".into())
	}

	/// Deletes the heading with its subtree; returns number of removed lines.
	pub fn delete_subtree(&mut self, i: usize) -> usize {
		let h = &self.headings[i];
		let (start, len) = (h.line, h.subtree_end - h.line);
		self.splice(start, len, vec![], true);
		len
	}

	/// Moves a subtree, adjusting heading levels. Returns the new index of the moved heading.
	pub fn move_subtree(&mut self, i: usize, pos: Pos) -> Result<usize, String> {
		let h = self.headings[i].clone();
		let target = match pos {
			Pos::Under { parent, .. } => Some(parent),
			Pos::After(s) | Pos::Before(s) => Some(s),
			Pos::Top { .. } => None,
		};
		if let Some(t) = target
			&& (t == i || self.is_descendant(t, i))
		{
			return Err("cannot move a heading relative to itself or its own descendant".into());
		}
		let target_line = target.map(|t| self.headings[t].line);
		let content_end = self.after_last_nonblank(h.line, h.subtree_end);
		let block: Vec<String> = self.lines[h.line..content_end].to_vec();
		let removed = h.subtree_end - h.line;

		self.splice(h.line, removed, vec![], true);
		let new_pos = match (pos, target_line) {
			(Pos::Top { first }, _) => Pos::Top { first },
			(p, Some(tl)) => {
				let tl = if tl > h.line { tl - removed } else { tl };
				let t = self
					.heading_at_line(tl)
					.ok_or("internal: move target lost")?;
				match p {
					Pos::Under { first, .. } => Pos::Under { parent: t, first },
					Pos::After(_) => Pos::After(t),
					_ => Pos::Before(t),
				}
			},
			_ => unreachable!(),
		};
		let (at, level) = self.insert_point(new_pos);
		let delta = level as isize - h.level as isize;
		let block: Vec<String> = block
			.into_iter()
			.map(|l| {
				if !is_heading_line(&l) {
					return l;
				}
				let stars = l.chars().take_while(|&c| c == '*').count();
				let n = (stars as isize + delta).max(1) as usize;
				format!("{}{}", "*".repeat(n), &l[stars..])
			})
			.collect();
		self.splice(at, 0, block, true);
		self.heading_at_line(at)
			.ok_or_else(|| "internal: moved heading not found".into())
	}
}
