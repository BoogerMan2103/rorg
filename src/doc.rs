//! Lossless org document model: the file is kept as lines, headings are indexes into them.

use crate::ts::{self, Timestamp};
use regex::Regex;
use serde::Serialize;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static HEADING_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^(\*+)[ \t]+(.*?)\s*$").unwrap());
static TAGS_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^(?:(.*?)([ \t]+))?((?::[\w@#%]+)+:)$").unwrap());
static PRIO_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[#([A-Za-z0-9])\]\s*").unwrap());
static PLANNING_LINE_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^\s*(SCHEDULED|DEADLINE|CLOSED):").unwrap());
static PLAN_ITEM_RE: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(SCHEDULED|DEADLINE|CLOSED):\s*([<\[][^<>\[\]\n]*[>\]])").unwrap()
});
static TODO_LINE_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"(?i)^#\+(?:SEQ_|TYP_)?TODO:\s*(.*)$").unwrap());
static FILETAGS_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"(?i)^#\+FILETAGS:\s*(.*)$").unwrap());
static DRAWER_START_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^\s*:([\w-]+):\s*$").unwrap());
static PROP_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^\s*:([^\s:]+?\+?):(?:[ \t]+(.*?))?\s*$").unwrap());
static CLOCK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*CLOCK:\s*(.*)$").unwrap());
static DURATION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"=>\s*(\d+):(\d{2})").unwrap());

pub const DEFAULT_KEYWORDS: &str =
	"TODO NEXT STARTED IN-PROGRESS WAIT WAITING HOLD SOMEDAY | DONE CANCELLED CANCELED";
pub const DEFAULT_PROTECTED_TAGS: &str = "crypt,secret";

#[derive(Debug, Clone, Serialize)]
pub struct Keywords {
	pub todo: Vec<String>,
	pub done: Vec<String>,
	pub source: String,
	/// Keywords matched by `find --todo` / `--done` (narrower than todo/done if configured).
	pub find_todo: Vec<String>,
	pub find_done: Vec<String>,
}

impl Keywords {
	fn parse_into(&mut self, spec: &str) {
		let clean = |t: &str| t.split('(').next().unwrap_or(t).to_string();
		let toks: Vec<&str> = spec.split_whitespace().collect();
		if let Some(bar) = toks.iter().position(|t| *t == "|") {
			self.todo.extend(toks[..bar].iter().map(|t| clean(t)));
			self.done.extend(toks[bar + 1..].iter().map(|t| clean(t)));
		} else if let Some((last, rest)) = toks.split_last() {
			self.todo.extend(rest.iter().map(|t| clean(t)));
			self.done.push(clean(last));
		}
	}

	pub fn from_spec(spec: &str, source: &str) -> Self {
		let mut k = Self {
			todo: vec![],
			done: vec![],
			source: source.to_string(),
			find_todo: vec![],
			find_done: vec![],
		};
		k.parse_into(spec);
		k.find_todo = k.todo.clone();
		k.find_done = k.done.clone();
		k
	}

	/// From `#+TODO:` lines of the file, else `$RORG_TODO`, else a broad default.
	/// `$RORG_FIND_TODO` / `$RORG_FIND_DONE` narrow what `find --todo` / `--done` match.
	fn detect(lines: &[String]) -> Self {
		let file_spec: Vec<String> = lines
			.iter()
			.filter_map(|l| TODO_LINE_RE.captures(l).map(|c| c[1].to_string()))
			.collect();
		let mut k = if !file_spec.is_empty() {
			Self::from_spec(&file_spec.join(" "), "file")
		} else {
			match std::env::var("RORG_TODO") {
				Ok(spec) if !spec.trim().is_empty() => Self::from_spec(&spec, "env RORG_TODO"),
				_ => Self::from_spec(DEFAULT_KEYWORDS, "default"),
			}
		};
		let env_list = |name: &str| -> Option<Vec<String>> {
			let v = std::env::var(name).ok()?;
			let list: Vec<String> = v.split_whitespace().map(str::to_string).collect();
			(!list.is_empty()).then_some(list)
		};
		if let Some(l) = env_list("RORG_FIND_TODO") {
			k.find_todo = l;
		}
		if let Some(l) = env_list("RORG_FIND_DONE") {
			k.find_done = l;
		}
		k
	}

	pub fn contains(&self, k: &str) -> bool {
		self.todo.iter().chain(&self.done).any(|x| x == k)
	}

	pub fn is_done(&self, k: &str) -> bool {
		self.done.iter().any(|x| x == k)
	}

	/// Case-insensitive lookup returning the canonical spelling.
	pub fn canonical(&self, k: &str) -> Option<String> {
		self.todo
			.iter()
			.chain(&self.done)
			.find(|x| x.eq_ignore_ascii_case(k))
			.cloned()
	}

	pub fn describe(&self) -> String {
		format!("{} | {}", self.todo.join(" "), self.done.join(" "))
	}
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Planning {
	pub scheduled: Option<Timestamp>,
	pub deadline: Option<Timestamp>,
	pub closed: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanKind {
	Scheduled,
	Deadline,
	Closed,
}

impl PlanKind {
	pub fn keyword(self) -> &'static str {
		match self {
			Self::Scheduled => "SCHEDULED",
			Self::Deadline => "DEADLINE",
			Self::Closed => "CLOSED",
		}
	}
}

impl Planning {
	pub fn get(&self, k: PlanKind) -> Option<&Timestamp> {
		match k {
			PlanKind::Scheduled => self.scheduled.as_ref(),
			PlanKind::Deadline => self.deadline.as_ref(),
			PlanKind::Closed => self.closed.as_ref(),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct Clock {
	pub start: Timestamp,
	pub minutes: Option<u32>,
	pub running: bool,
}

#[derive(Debug, Clone)]
pub struct Heading {
	/// 0-based index of the heading line.
	pub line: usize,
	/// End (exclusive) of own section: next heading of any level.
	pub section_end: usize,
	/// End (exclusive) of the whole subtree.
	pub subtree_end: usize,
	pub level: usize,
	pub keyword: Option<String>,
	pub priority: Option<char>,
	pub title: String,
	pub tags: Vec<String>,
	/// Column where the tags string ends, when tags are aligned with extra whitespace.
	pub tags_align_end: Option<usize>,
	pub parent: Option<usize>,
	pub children: Vec<usize>,
	/// 1-based positional path, e.g. [2, 1] for "2.1".
	pub path: Vec<usize>,
	pub planning: Planning,
	pub planning_end: usize,
	pub props_range: Option<Range<usize>>,
	pub properties: Vec<(String, String)>,
	/// First line of free text (after planning, properties and leading drawers).
	pub body_start: usize,
	/// Timestamps in title and body (not planning, not CLOCK lines).
	pub timestamps: Vec<Timestamp>,
	pub clocks: Vec<Clock>,
}

impl Heading {
	pub fn id(&self) -> String {
		self.path
			.iter()
			.map(|n| n.to_string())
			.collect::<Vec<_>>()
			.join(".")
	}

	pub fn prop(&self, key: &str) -> Option<&str> {
		self.properties
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case(key))
			.map(|(_, v)| v.as_str())
	}

	pub fn clock_minutes(&self) -> u32 {
		self.clocks.iter().filter_map(|c| c.minutes).sum()
	}
}

/// Recorded line splice, used for printing diffs.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
	pub line: usize,
	pub removed: Vec<String>,
	pub added: Vec<String>,
	/// Large structural change (delete/move): print a summary instead of lines.
	pub quiet: bool,
}

pub struct Doc {
	pub path: PathBuf,
	pub lines: Vec<String>,
	pub trailing_newline: bool,
	pub keywords: Keywords,
	pub filetags: Vec<String>,
	pub protected_tags: Vec<String>,
	pub ru_days: bool,
	pub headings: Vec<Heading>,
	pub changes: Vec<Change>,
}

impl Doc {
	pub fn load(path: &Path) -> Result<Self, String> {
		let text =
			fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
		Ok(Self::parse(path, &text))
	}

	pub fn parse(path: &Path, text: &str) -> Self {
		let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
		let trailing_newline = text.ends_with('\n');
		if trailing_newline || text.is_empty() {
			lines.pop();
		}
		let protected_tags = std::env::var("RORG_PROTECT_TAGS")
			.unwrap_or_else(|_| DEFAULT_PROTECTED_TAGS.to_string())
			.split(',')
			.map(|s| s.trim().to_string())
			.filter(|s| !s.is_empty())
			.collect();
		let mut doc = Self {
			path: path.to_path_buf(),
			keywords: Keywords::detect(&lines),
			ru_days: ts::detect_ru(text),
			lines,
			trailing_newline,
			filetags: vec![],
			protected_tags,
			headings: vec![],
			changes: vec![],
		};
		doc.reparse();
		doc
	}

	pub fn render(&self) -> String {
		let mut s = self.lines.join("\n");
		if self.trailing_newline {
			s.push('\n');
		}
		s
	}

	/// Atomic write: temp file in the same directory, then rename.
	pub fn save(&self) -> Result<(), String> {
		let dir = self
			.path
			.parent()
			.filter(|p| !p.as_os_str().is_empty())
			.unwrap_or(Path::new("."));
		let name = self
			.path
			.file_name()
			.ok_or("bad file path")?
			.to_string_lossy();
		let tmp = dir.join(format!(".{name}.rorg-tmp"));
		fs::write(&tmp, self.render())
			.map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
		if let Ok(meta) = fs::metadata(&self.path) {
			let _ = fs::set_permissions(&tmp, meta.permissions());
		}
		fs::rename(&tmp, &self.path)
			.map_err(|e| format!("cannot replace {}: {e}", self.path.display()))
	}

	pub fn reparse(&mut self) {
		self.filetags = self
			.lines
			.iter()
			.take_while(|l| !HEADING_RE.is_match(l))
			.filter_map(|l| FILETAGS_RE.captures(l))
			.flat_map(|c| {
				c[1].split(':')
					.map(|t| t.trim().to_string())
					.collect::<Vec<_>>()
			})
			.filter(|t| !t.is_empty())
			.collect();
		self.headings = parse_headings(&self.lines, &self.keywords);
	}

	// ---------- navigation ----------

	pub fn breadcrumb(&self, i: usize) -> Vec<&str> {
		let mut out = vec![];
		let mut p = self.headings[i].parent;
		while let Some(j) = p {
			out.push(self.headings[j].title.as_str());
			p = self.headings[j].parent;
		}
		out.reverse();
		out
	}

	/// Own tags plus inherited ones (file tags, ancestors), deduplicated.
	pub fn all_tags(&self, i: usize) -> Vec<String> {
		let mut chain = vec![i];
		let mut p = self.headings[i].parent;
		while let Some(j) = p {
			chain.push(j);
			p = self.headings[j].parent;
		}
		let mut out: Vec<String> = self.filetags.clone();
		for j in chain.into_iter().rev() {
			for t in &self.headings[j].tags {
				if !out.contains(t) {
					out.push(t.clone());
				}
			}
		}
		out
	}

	pub fn is_protected(&self, i: usize) -> bool {
		self.all_tags(i).iter().any(|t| {
			self.protected_tags
				.iter()
				.any(|p| p.eq_ignore_ascii_case(t))
		})
	}

	/// Heading whose section contains the 0-based line.
	pub fn heading_at_line(&self, line: usize) -> Option<usize> {
		self.headings.iter().rposition(|h| h.line <= line)
	}

	pub fn is_descendant(&self, i: usize, ancestor: usize) -> bool {
		let mut p = self.headings[i].parent;
		while let Some(j) = p {
			if j == ancestor {
				return true;
			}
			p = self.headings[j].parent;
		}
		false
	}

	/// Index just past the last non-blank line in [from, to), or `from` if all blank.
	pub fn after_last_nonblank(&self, from: usize, to: usize) -> usize {
		(from..to)
			.rev()
			.find(|&j| !self.lines[j].trim().is_empty())
			.map_or(from, |j| j + 1)
	}

	/// Free-text body range, without trailing blank lines.
	pub fn body_range(&self, i: usize) -> Range<usize> {
		let h = &self.headings[i];
		h.body_start..self.after_last_nonblank(h.body_start, h.section_end)
	}

	/// Resolve a selector to a heading index.
	///
	/// `1.2.3` positional path · `L42` heading containing line 42 · `id:X` :ID:/:CUSTOM_ID:
	/// · `@text` unique title substring (case-insensitive) · `@=Title` exact title.
	pub fn resolve(&self, sel: &str) -> Result<usize, String> {
		let s = sel.trim();
		if let Some(n) = s
			.strip_prefix(['L', 'l'])
			.and_then(|n| n.parse::<usize>().ok())
		{
			if n == 0 || n > self.lines.len() {
				return Err(format!(
					"line {n} is out of range (1..{})",
					self.lines.len()
				));
			}
			return self
				.heading_at_line(n - 1)
				.ok_or_else(|| format!("line {n} is before the first heading"));
		}
		if let Some(id) = s.strip_prefix("id:") {
			return self.unique(
				|h| h.prop("ID") == Some(id) || h.prop("CUSTOM_ID") == Some(id),
				&format!("id:{id}"),
			);
		}
		if let Some(t) = s.strip_prefix("@=") {
			return self.unique(|h| h.title == t, sel);
		}
		if let Some(t) = s.strip_prefix('@') {
			return self.by_title(t, sel);
		}
		if !s.is_empty()
			&& s.split('.')
				.all(|p| p.parse::<usize>().is_ok_and(|n| n > 0))
		{
			let path: Vec<usize> = s.split('.').map(|p| p.parse().unwrap()).collect();
			return self
				.headings
				.iter()
				.position(|h| h.path == path)
				.ok_or_else(|| format!("no heading at path {s} in {}", self.path.display()));
		}
		self.by_title(s, sel).map_err(|e| {
			format!("{e}\n(selectors: 1.2.3, L42, id:XYZ, @title-substring, @=Exact title)")
		})
	}

	/// Unique case-insensitive title substring. If nothing matches, retries with the text
	/// parsed as a pasted heading line (stars, keyword, priority, tags stripped).
	fn by_title(&self, t: &str, sel: &str) -> Result<usize, String> {
		let raw = t.trim().to_lowercase();
		let first = self.unique(|h| h.title.to_lowercase().contains(&raw), sel);
		if !matches!(&first, Err(e) if e.starts_with("no heading")) {
			return first;
		}
		let stripped = t.trim().trim_start_matches('*').trim_start();
		let title = parse_heading_text(stripped, &self.keywords)
			.2
			.to_lowercase();
		if title.is_empty() || title == raw {
			return first;
		}
		self.unique(|h| h.title.to_lowercase().contains(&title), sel)
	}

	fn unique(&self, f: impl Fn(&Heading) -> bool, what: &str) -> Result<usize, String> {
		let found: Vec<usize> = (0..self.headings.len())
			.filter(|&i| f(&self.headings[i]))
			.collect();
		match found.as_slice() {
			[i] => Ok(*i),
			[] => Err(format!("no heading matches '{what}'")),
			many => {
				let list: Vec<String> = many
					.iter()
					.take(10)
					.map(|&i| {
						format!(
							"  {}  L{}  {}",
							self.headings[i].id(),
							self.headings[i].line + 1,
							self.headings[i].title
						)
					})
					.collect();
				Err(format!(
					"'{what}' is ambiguous ({} matches):\n{}",
					many.len(),
					list.join("\n")
				))
			},
		}
	}
}

/// Splits heading text (after the stars) into keyword, priority, title, tags.
pub fn parse_heading_text(
	text: &str,
	kw: &Keywords,
) -> (
	Option<String>,
	Option<char>,
	String,
	Vec<String>,
	Option<usize>,
) {
	let mut rest = text.trim_end();
	let mut tags = vec![];
	let mut align = false;
	if let Some(c) = TAGS_RE.captures(rest) {
		tags = c[3]
			.split(':')
			.filter(|t| !t.is_empty())
			.map(str::to_string)
			.collect();
		align = c.get(2).is_some_and(|g| g.as_str().len() > 1);
		rest = c.get(1).map_or("", |g| g.as_str());
	}
	let mut keyword = None;
	if let Some(first) = rest.split_whitespace().next()
		&& kw.contains(first)
	{
		keyword = Some(first.to_string());
		rest = rest[first.len()..].trim_start();
	}
	let mut priority = None;
	if let Some(c) = PRIO_RE.captures(rest) {
		priority = c[1].chars().next();
		rest = &rest[c[0].len()..];
	}
	(
		keyword,
		priority,
		rest.trim().to_string(),
		tags,
		if align { Some(0) } else { None },
	)
}

pub fn build_heading_line(
	level: usize,
	keyword: Option<&str>,
	priority: Option<char>,
	title: &str,
	tags: &[String],
	align_end: Option<usize>,
) -> String {
	let mut s = "*".repeat(level);
	for part in [
		keyword.map(str::to_string),
		priority.map(|p| format!("[#{p}]")),
		Some(title.to_string()),
	]
	.into_iter()
	.flatten()
	.filter(|p| !p.is_empty())
	{
		s.push(' ');
		s.push_str(&part);
	}
	if !tags.is_empty() {
		let t = format!(":{}:", tags.join(":"));
		let width = s.chars().count();
		let tlen = t.chars().count();
		let pad = match align_end {
			Some(end) if end > width + tlen => end - width - tlen,
			_ => 1,
		};
		s.push_str(&" ".repeat(pad));
		s.push_str(&t);
	}
	s
}

fn parse_headings(lines: &[String], kw: &Keywords) -> Vec<Heading> {
	let mut hs: Vec<Heading> = vec![];
	let mut stack: Vec<usize> = vec![];
	let mut top = 0;
	for (i, line) in lines.iter().enumerate() {
		let Some(c) = HEADING_RE.captures(line) else {
			continue;
		};
		let level = c[1].len();
		let (keyword, priority, title, tags, align) = parse_heading_text(&c[2], kw);
		while let Some(&t) = stack.last()
			&& hs[t].level >= level
		{
			stack.pop();
		}
		let idx = hs.len();
		let parent = stack.last().copied();
		let path = match parent {
			Some(p) => {
				hs[p].children.push(idx);
				let mut v = hs[p].path.clone();
				v.push(hs[p].children.len());
				v
			},
			None => {
				top += 1;
				vec![top]
			},
		};
		hs.push(Heading {
			line: i,
			section_end: 0,
			subtree_end: 0,
			level,
			keyword,
			priority,
			title,
			tags,
			tags_align_end: align.map(|_| line.trim_end().chars().count()),
			parent,
			children: vec![],
			path,
			planning: Planning::default(),
			planning_end: i + 1,
			props_range: None,
			properties: vec![],
			body_start: i + 1,
			timestamps: vec![],
			clocks: vec![],
		});
		stack.push(idx);
	}

	let n = hs.len();
	for i in 0..n {
		hs[i].section_end = hs.get(i + 1).map_or(lines.len(), |h| h.line);
		let lvl = hs[i].level;
		hs[i].subtree_end = hs[i + 1..]
			.iter()
			.find(|h| h.level <= lvl)
			.map_or(lines.len(), |h| h.line);
		parse_section(&mut hs[i], lines);
	}
	hs
}

fn find_drawer_end(lines: &[String], from: usize, to: usize) -> Option<usize> {
	(from..to).find(|&k| lines[k].trim().eq_ignore_ascii_case(":END:"))
}

fn parse_section(h: &mut Heading, lines: &[String]) {
	let end = h.section_end;
	let mut j = h.line + 1;
	while j < end && PLANNING_LINE_RE.is_match(&lines[j]) {
		for c in PLAN_ITEM_RE.captures_iter(&lines[j]) {
			let t = ts::find_all(&c[2]).into_iter().next();
			match &c[1] {
				"SCHEDULED" => h.planning.scheduled = t,
				"DEADLINE" => h.planning.deadline = t,
				_ => h.planning.closed = t,
			}
		}
		j += 1;
	}
	h.planning_end = j;

	if j < end
		&& lines[j].trim().eq_ignore_ascii_case(":PROPERTIES:")
		&& let Some(k) = find_drawer_end(lines, j + 1, end)
	{
		for l in &lines[j + 1..k] {
			if let Some(c) = PROP_RE.captures(l) {
				h.properties.push((
					c[1].to_string(),
					c.get(2).map_or("", |g| g.as_str()).to_string(),
				));
			}
		}
		h.props_range = Some(j..k + 1);
		j = k + 1;
	}
	while j < end
		&& DRAWER_START_RE.is_match(&lines[j])
		&& let Some(k) = find_drawer_end(lines, j + 1, end)
	{
		j = k + 1;
	}
	h.body_start = j;

	h.timestamps = ts::find_all(&h.title);
	for l in &lines[h.line + 1..end] {
		if let Some(c) = CLOCK_RE.captures(l) {
			let all = ts::find_all(&c[1]);
			if let Some(start) = all.first().cloned() {
				let minutes = DURATION_RE
					.captures(&c[1])
					.and_then(|d| Some(d[1].parse::<u32>().ok()? * 60 + d[2].parse::<u32>().ok()?));
				h.clocks.push(Clock {
					start,
					minutes,
					running: all.len() < 2,
				});
			}
		}
	}
	for l in &lines[h.planning_end..end] {
		if !CLOCK_RE.is_match(l) {
			h.timestamps.extend(ts::find_all(l));
		}
	}
}

pub fn is_heading_line(l: &str) -> bool {
	HEADING_RE.is_match(l)
}

pub fn prop_re() -> &'static Regex {
	&PROP_RE
}
