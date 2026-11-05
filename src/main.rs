mod doc;
mod edit;
mod ts;

#[cfg(test)]
mod tests;

use chrono::{Local, NaiveDate, NaiveDateTime};
use clap::{Args, Parser, Subcommand};
use doc::{Doc, PlanKind};
use edit::{NewEntry, Pos, StatusOutcome};
use regex::RegexBuilder;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use ts::{DateFilter, DateRange};

const ABOUT: &str = "Query and edit org-mode files piece by piece, without loading whole files.

Selectors (SEL): 1.2.3 = positional path (as printed by outline/find) · L42 = heading containing line 42 ·
id:XYZ = :ID:/:CUSTOM_ID: property · @text = unique title substring · @=Title = exact title.
Dates: YYYY-MM-DD, today, tomorrow, yesterday, +3d, -1w, +1m, fri. Ranges: A..B, ..B, A.., overdue, week.
Positional ids shift after add/move/delete — re-query (every write prints the new id),
or guard writes with --expect TEXT (aborts unless the target title contains TEXT).
Bodies of headings tagged crypt/secret (or inheriting them) are hidden unless --reveal ($RORG_PROTECT_TAGS).
TODO keywords come from #+TODO lines, else $RORG_TODO, else a broad default (see `rorg info`).
$RORG_FIND_TODO / $RORG_FIND_DONE (space-separated) narrow what find --todo / --done match.";

#[derive(Parser)]
#[command(name = "rorg", version, about = "Agent-friendly org-mode query/edit CLI", long_about = ABOUT)]
struct Cli {
	/// JSON output (outline, find, show, grep, writes)
	#[arg(long, global = true)]
	json: bool,
	/// For write commands: print the diff, don't save
	#[arg(long, global = true)]
	dry_run: bool,
	/// For write commands: abort unless the target heading's title contains this text
	#[arg(long, global = true)]
	expect: Option<String>,
	#[command(subcommand)]
	cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
	/// Overview: size, TODO keywords, keyword counts, tags
	Info {
		/// Org files or directories (searched recursively for *.org)
		#[arg(required = true)]
		paths: Vec<PathBuf>,
	},
	/// Heading tree (no bodies)
	Outline {
		/// Org files or directories (searched recursively for *.org)
		#[arg(required = true)]
		paths: Vec<PathBuf>,
		/// Max heading level to show
		#[arg(short, long)]
		depth: Option<usize>,
		/// Only the subtree of this heading (single file)
		#[arg(long)]
		under: Option<String>,
	},
	/// Find headings by keyword, text, tags, dates, properties
	Find(FindArgs),
	/// One heading: metadata, children; body only with --body
	Show {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// Print the free-text body (without drawers)
		#[arg(long)]
		body: bool,
		/// Print raw lines of the section (heading, planning, drawers, body)
		#[arg(long)]
		raw: bool,
		/// With --raw: include the whole subtree
		#[arg(long)]
		subtree: bool,
		/// Max body/raw lines to print
		#[arg(long, default_value_t = 200)]
		max_lines: usize,
		/// Show body even for protected (crypt/secret) headings
		#[arg(long)]
		reveal: bool,
	},
	/// Search lines by regex; prints line number, owning heading id, the line
	Grep {
		/// Regex; case-insensitive unless it contains uppercase (smart case)
		pattern: String,
		/// Org files or directories (searched recursively for *.org)
		#[arg(required = true)]
		paths: Vec<PathBuf>,
		/// Case-sensitive (default: smart case)
		#[arg(short = 's', long)]
		case_sensitive: bool,
		/// Max matches to print (total is still reported)
		#[arg(long, default_value_t = 50)]
		limit: usize,
		/// Include matches inside protected (crypt/secret) headings
		#[arg(long)]
		reveal: bool,
	},
	/// Scheduled/deadline/timestamped open items by day, plus overdue
	Agenda {
		/// Org files or directories (searched recursively for *.org)
		#[arg(required = true)]
		paths: Vec<PathBuf>,
		/// Date range, e.g. today..+6d, week, 2026-11-01..2026-11-30
		#[arg(short, long, default_value = "today..+6d")]
		range: String,
		/// Include done items
		#[arg(long)]
		all: bool,
	},
	/// Clocked time per heading
	Clock {
		/// Org files or directories (searched recursively for *.org)
		#[arg(required = true)]
		paths: Vec<PathBuf>,
		/// Only CLOCK entries starting in this range
		#[arg(short, long)]
		range: Option<String>,
	},
	/// Set TODO keyword (`none` to clear). Adds/removes CLOSED like org-log-done; handles repeaters
	Status {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// New keyword (case-insensitive), or `none` to clear
		keyword: String,
		/// Don't touch CLOSED / LAST_REPEAT
		#[arg(long)]
		no_log: bool,
	},
	/// Replace heading title (keyword, priority, tags kept)
	Title {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// New title
		text: String,
	},
	/// Change tags
	Tag {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// Tags to add (comma-separated)
		#[arg(long, value_delimiter = ',')]
		add: Vec<String>,
		/// Tags to remove (comma-separated)
		#[arg(long, value_delimiter = ',')]
		rm: Vec<String>,
		/// Replace all own tags (comma-separated, empty string clears)
		#[arg(long)]
		set: Option<String>,
	},
	/// Set priority letter, or `none`
	Priority {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// Priority letter (A, B, C…) or `none`
		value: String,
	},
	/// Set SCHEDULED (`none` removes). WHEN: DATE [HH:MM] [+1w] [-2d] or raw <...>
	Schedule {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// DATE [HH:MM[-HH:MM]] [+1w repeater] [-2d warning], a raw <…> timestamp, or `none` to remove
		when: String,
	},
	/// Set DEADLINE (`none` removes)
	Deadline {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// DATE [HH:MM[-HH:MM]] [+1w repeater] [-2d warning], a raw <…> timestamp, or `none` to remove
		when: String,
	},
	/// Set a property in the :PROPERTIES: drawer (omit VALUE with --unset to remove)
	Prop {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// Property name
		key: String,
		/// Property value
		value: Option<String>,
		/// Remove the property (drawer is removed when it becomes empty)
		#[arg(long)]
		unset: bool,
	},
	/// Append to (default) or replace the free-text body. Without TEXT reads stdin
	Body {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		/// Text; omit or `-` to read stdin
		text: Option<String>,
		/// Add after the existing body (default)
		#[arg(long, conflicts_with = "set")]
		append: bool,
		/// Replace the body (empty text clears); planning and drawers are kept
		#[arg(long)]
		set: bool,
	},
	/// Insert a new heading
	Add {
		/// Org file
		file: PathBuf,
		#[command(flatten)]
		pos: PosArgs,
		/// Heading title
		#[arg(long)]
		title: String,
		/// TODO keyword (case-insensitive)
		#[arg(long)]
		status: Option<String>,
		/// Tags (comma-separated)
		#[arg(long, value_delimiter = ',')]
		tags: Vec<String>,
		/// Priority letter
		#[arg(long)]
		priority: Option<char>,
		/// SCHEDULED: DATE [HH:MM] [+1w] [-2d], or a raw <…> timestamp
		#[arg(long)]
		scheduled: Option<String>,
		/// DEADLINE: same format as --scheduled
		#[arg(long)]
		deadline: Option<String>,
		/// Body text, `-` reads stdin
		#[arg(long)]
		body: Option<String>,
	},
	/// Delete a heading with its whole subtree
	Delete {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
	},
	/// Move a subtree (refile within the file); levels are adjusted
	Move {
		/// Org file
		file: PathBuf,
		/// Target heading: 1.2.3 | L42 | id:X | @title-substring | @=Exact title
		sel: String,
		#[command(flatten)]
		pos: PosArgs,
	},
}

#[derive(Args)]
#[group(required = true, multiple = false, id = "position")]
struct PosArgs {
	/// As child of SEL (last child, or first with --first)
	#[arg(long, group = "position")]
	under: Option<String>,
	/// As next sibling of SEL
	#[arg(long, group = "position")]
	after: Option<String>,
	/// As previous sibling of SEL
	#[arg(long, group = "position")]
	before: Option<String>,
	/// As top-level heading (end of file, or start with --first)
	#[arg(long, group = "position")]
	top: bool,
	/// With --under / --top: insert as the first child / at the top instead of last
	#[arg(long)]
	first: bool,
}

#[derive(Args)]
struct FindArgs {
	/// Org files or directories (searched recursively for *.org)
	#[arg(required = true)]
	paths: Vec<PathBuf>,
	/// Keywords, comma-separated; `none` = no keyword
	#[arg(short, long, value_delimiter = ',')]
	status: Vec<String>,
	/// Any not-done keyword (or $RORG_FIND_TODO list)
	#[arg(long)]
	todo: bool,
	/// Any done keyword (or $RORG_FIND_DONE list)
	#[arg(long)]
	done: bool,
	/// Title contains text (case-insensitive)
	#[arg(short, long)]
	text: Option<String>,
	/// Treat --text as a regex
	#[arg(long)]
	regex: bool,
	/// --text also matches body lines (protected bodies skipped)
	#[arg(long)]
	in_body: bool,
	/// Has all these tags (inherited tags count)
	#[arg(long = "tag", value_delimiter = ',')]
	tags: Vec<String>,
	/// Has none of these tags (comma-separated, inherited tags count)
	#[arg(long = "not-tag", value_delimiter = ',')]
	not_tags: Vec<String>,
	/// Exact heading level (1 = top level)
	#[arg(long)]
	level: Option<usize>,
	/// Only descendants of SEL (single file)
	#[arg(long)]
	under: Option<String>,
	/// Priority letter
	#[arg(long)]
	priority: Option<char>,
	/// Has SCHEDULED (no value), or RANGE | any | none. Put after the file paths
	#[arg(long, num_args = 0..=1, default_missing_value = "any")]
	scheduled: Option<String>,
	/// Has DEADLINE (no value), or RANGE | any | none
	#[arg(long, num_args = 0..=1, default_missing_value = "any")]
	deadline: Option<String>,
	/// Has CLOSED (no value), or RANGE | any | none
	#[arg(long, num_args = 0..=1, default_missing_value = "any")]
	closed: Option<String>,
	/// SCHEDULED, DEADLINE or any active timestamp in title/body falls in RANGE
	#[arg(long)]
	date: Option<String>,
	/// KEY or KEY=VALUE
	#[arg(long)]
	prop: Vec<String>,
	/// Order results by date (undated last) instead of file order
	#[arg(long, value_enum)]
	sort: Option<SortBy>,
	/// Max results to print (total is still reported)
	#[arg(long, default_value_t = 50)]
	limit: usize,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SortBy {
	Scheduled,
	Deadline,
	Closed,
}

type Res<T = ()> = Result<T, String>;

struct Ctx {
	json: bool,
	dry_run: bool,
	expect: Option<String>,
	today: NaiveDate,
	now: NaiveDateTime,
}

fn main() {
	let cli = Cli::parse();
	let now = Local::now().naive_local();
	let ctx = Ctx {
		json: cli.json,
		dry_run: cli.dry_run,
		expect: cli.expect,
		today: now.date(),
		now,
	};
	if let Err(e) = run(cli.cmd, &ctx) {
		eprintln!("error: {e}");
		std::process::exit(1);
	}
}

fn run(cmd: Cmd, ctx: &Ctx) -> Res {
	match cmd {
		Cmd::Info { paths } => cmd_info(&load_all(&paths)?),
		Cmd::Outline {
			paths,
			depth,
			under,
		} => cmd_outline(&load_all(&paths)?, depth, under, ctx),
		Cmd::Find(a) => cmd_find(a, ctx),
		Cmd::Show {
			file,
			sel,
			body,
			raw,
			subtree,
			max_lines,
			reveal,
		} => cmd_show(
			&Doc::load(&file)?,
			&sel,
			body,
			raw,
			subtree,
			max_lines,
			reveal,
			ctx,
		),
		Cmd::Grep {
			pattern,
			paths,
			case_sensitive,
			limit,
			reveal,
		} => cmd_grep(
			&load_all(&paths)?,
			&pattern,
			case_sensitive,
			limit,
			reveal,
			ctx,
		),
		Cmd::Agenda { paths, range, all } => cmd_agenda(&load_all(&paths)?, &range, all, ctx),
		Cmd::Clock { paths, range } => cmd_clock(&load_all(&paths)?, range.as_deref(), ctx),
		Cmd::Status {
			file,
			sel,
			keyword,
			no_log,
		} => write(&file, &sel, ctx, |d, i| {
			let k = if keyword.eq_ignore_ascii_case("none") {
				None
			} else {
				Some(d.keywords.canonical(&keyword).unwrap_or(keyword.clone()))
			};
			let out = d.set_keyword(i, k, !no_log, ctx.now)?;
			if let StatusOutcome::Repeated = out {
				eprintln!(
					"note: repeating task — dates shifted, keyword kept (org repeater semantics)"
				);
			}
			Ok(i)
		}),
		Cmd::Title { file, sel, text } => {
			write(&file, &sel, ctx, |d, i| d.set_title(i, &text).map(|_| i))
		},
		Cmd::Tag {
			file,
			sel,
			add,
			rm,
			set,
		} => write(&file, &sel, ctx, |d, i| {
			let mut tags = match &set {
				Some(s) => s
					.split([',', ':'])
					.map(|t| t.trim().to_string())
					.filter(|t| !t.is_empty())
					.collect(),
				None => d.headings[i].tags.clone(),
			};
			tags.retain(|t| !rm.contains(t));
			for t in &add {
				if !tags.contains(t) {
					tags.push(t.clone());
				}
			}
			d.set_tags(i, tags).map(|_| i)
		}),
		Cmd::Priority { file, sel, value } => write(&file, &sel, ctx, |d, i| {
			let p = match value.as_str() {
				"none" => None,
				v if v.chars().count() == 1 && v.chars().all(|c| c.is_ascii_alphanumeric()) => {
					v.chars().next().map(|c| c.to_ascii_uppercase())
				},
				v => return Err(format!("priority must be one letter or `none`, got '{v}'")),
			};
			d.set_priority(i, p).map(|_| i)
		}),
		Cmd::Schedule { file, sel, when } => {
			write_planning(&file, &sel, PlanKind::Scheduled, &when, ctx)
		},
		Cmd::Deadline { file, sel, when } => {
			write_planning(&file, &sel, PlanKind::Deadline, &when, ctx)
		},
		Cmd::Prop {
			file,
			sel,
			key,
			value,
			unset,
		} => write(&file, &sel, ctx, |d, i| {
			match (unset, &value) {
				(true, _) => d.set_property(i, &key, None)?,
				(false, Some(v)) => d.set_property(i, &key, Some(v))?,
				(false, None) => return Err("give a VALUE or --unset".into()),
			}
			Ok(i)
		}),
		Cmd::Body {
			file,
			sel,
			text,
			append: _,
			set,
		} => write(&file, &sel, ctx, |d, i| {
			let text = stdin_or(text.as_deref().unwrap_or("-"))?;
			if set {
				d.set_body(i, &text)?;
			} else {
				d.append_body(i, &text)?;
			}
			Ok(i)
		}),
		Cmd::Add {
			file,
			pos,
			title,
			status,
			tags,
			priority,
			scheduled,
			deadline,
			body,
		} => {
			let mut d = Doc::load(&file)?;
			let pos = resolve_pos(&d, &pos)?;
			let when = |w: &Option<String>| {
				w.as_deref()
					.map(|w| ts::parse_when(w, ctx.today, d.ru_days))
					.transpose()
			};
			let body = body.as_deref().map(stdin_or).transpose()?;
			let entry = NewEntry {
				title: &title,
				keyword: status.map(|k| d.keywords.canonical(&k).unwrap_or(k)),
				priority: priority.map(|c| c.to_ascii_uppercase()),
				tags,
				scheduled: when(&scheduled)?,
				deadline: when(&deadline)?,
				body: body.as_deref(),
			};
			let i = d.add_heading(pos, entry)?;
			finish_write(&d, Some(i), ctx)
		},
		Cmd::Delete { file, sel } => {
			let mut d = Doc::load(&file)?;
			let i = resolve_checked(&d, &sel, ctx)?;
			let h = d.headings[i].clone();
			let descendants = d
				.headings
				.iter()
				.filter(|o| o.line > h.line && o.line < h.subtree_end)
				.count();
			let n = d.delete_subtree(i);
			println!(
				"deleted {} L{} '{}' with {descendants} descendant heading(s), {n} line(s)",
				h.id(),
				h.line + 1,
				h.title
			);
			finish_write(&d, None, ctx)
		},
		Cmd::Move { file, sel, pos } => {
			let mut d = Doc::load(&file)?;
			let i = resolve_checked(&d, &sel, ctx)?;
			let old = d.headings[i].id();
			let pos = resolve_pos(&d, &pos)?;
			let ni = d.move_subtree(i, pos)?;
			println!("moved {old} -> {}", d.headings[ni].id());
			finish_write(&d, Some(ni), ctx)
		},
	}
}

// ---------- loading ----------

fn collect_org(p: &Path, out: &mut Vec<PathBuf>) -> Res {
	if p.is_dir() {
		let mut entries: Vec<PathBuf> = std::fs::read_dir(p)
			.map_err(|e| format!("cannot read dir {}: {e}", p.display()))?
			.filter_map(|e| e.ok().map(|e| e.path()))
			.filter(|e| {
				!e.file_name()
					.is_some_and(|n| n.to_string_lossy().starts_with('.'))
			})
			.collect();
		entries.sort();
		for e in entries {
			if e.is_dir() || (e.is_file() && e.extension().is_some_and(|x| x == "org")) {
				collect_org(&e, out)?;
			}
		}
		Ok(())
	} else if p.exists() {
		out.push(p.to_path_buf());
		Ok(())
	} else {
		Err(format!("no such file: {}", p.display()))
	}
}

fn load_all(paths: &[PathBuf]) -> Res<Vec<Doc>> {
	let mut files = vec![];
	for p in paths {
		collect_org(p, &mut files)?;
	}
	files.iter().map(|f| Doc::load(f)).collect()
}

fn stdin_or(t: &str) -> Res<String> {
	if t != "-" {
		return Ok(t.to_string());
	}
	if std::io::stdin().is_terminal() {
		return Err("no text given and stdin is a terminal; pass TEXT or pipe it in".into());
	}
	let mut s = String::new();
	std::io::stdin()
		.read_to_string(&mut s)
		.map_err(|e| format!("stdin: {e}"))?;
	Ok(s)
}

fn resolve_pos(d: &Doc, p: &PosArgs) -> Res<Pos> {
	Ok(if let Some(s) = &p.under {
		Pos::Under {
			parent: d.resolve(s)?,
			first: p.first,
		}
	} else if let Some(s) = &p.after {
		Pos::After(d.resolve(s)?)
	} else if let Some(s) = &p.before {
		Pos::Before(d.resolve(s)?)
	} else {
		Pos::Top { first: p.first }
	})
}

// ---------- output helpers ----------

#[derive(Serialize)]
struct HeadingOut {
	file: String,
	id: String,
	line: usize,
	level: usize,
	keyword: Option<String>,
	priority: Option<char>,
	title: String,
	tags: Vec<String>,
	inherited_tags: Vec<String>,
	scheduled: Option<String>,
	deadline: Option<String>,
	closed: Option<String>,
	parents: Vec<String>,
	children: usize,
	protected: bool,
}

fn heading_out(d: &Doc, i: usize) -> HeadingOut {
	let h = &d.headings[i];
	HeadingOut {
		file: d.path.display().to_string(),
		id: h.id(),
		line: h.line + 1,
		level: h.level,
		keyword: h.keyword.clone(),
		priority: h.priority,
		title: h.title.clone(),
		tags: h.tags.clone(),
		inherited_tags: d
			.all_tags(i)
			.into_iter()
			.filter(|t| !h.tags.contains(t))
			.collect(),
		scheduled: h.planning.scheduled.as_ref().map(|t| t.raw.clone()),
		deadline: h.planning.deadline.as_ref().map(|t| t.raw.clone()),
		closed: h.planning.closed.as_ref().map(|t| t.raw.clone()),
		parents: d.breadcrumb(i).into_iter().map(str::to_string).collect(),
		children: h.children.len(),
		protected: d.is_protected(i),
	}
}

/// One compact line: `[file:]ID  L12  KW [#A] Title  :tags:  sched:… dl:… closed:…`
fn heading_line(d: &Doc, i: usize, prefix: bool, crumbs: bool) -> String {
	let h = &d.headings[i];
	let mut s = String::new();
	if prefix {
		s.push_str(&format!("{}:", d.path.display()));
	}
	s.push_str(&format!("{}  L{}  ", h.id(), h.line + 1));
	if let Some(k) = &h.keyword {
		s.push_str(k);
		s.push(' ');
	}
	if let Some(p) = h.priority {
		s.push_str(&format!("[#{p}] "));
	}
	s.push_str(&h.title);
	if !h.tags.is_empty() {
		s.push_str(&format!("  :{}:", h.tags.join(":")));
	}
	for (label, t) in [
		("sched", &h.planning.scheduled),
		("dl", &h.planning.deadline),
		("closed", &h.planning.closed),
	] {
		if let Some(t) = t {
			s.push_str(&format!("  {label}:{}", t.short()));
		}
	}
	if crumbs && h.parent.is_some() {
		s.push_str(&format!("  (in: {})", d.breadcrumb(i).join(" > ")));
	}
	s
}

fn print_json(v: &impl Serialize) -> Res {
	println!(
		"{}",
		serde_json::to_string_pretty(v).map_err(|e| e.to_string())?
	);
	Ok(())
}

fn truncate(s: &str, n: usize) -> String {
	if s.chars().count() <= n {
		s.to_string()
	} else {
		format!("{}…", s.chars().take(n).collect::<String>())
	}
}

// ---------- read commands ----------

fn cmd_info(docs: &[Doc]) -> Res {
	for d in docs {
		let max_depth = d.headings.iter().map(|h| h.level).max().unwrap_or(0);
		println!(
			"{}: {} lines, {} headings, max depth {max_depth}",
			d.path.display(),
			d.lines.len(),
			d.headings.len()
		);
		println!(
			"  keywords ({}): {}",
			d.keywords.source,
			d.keywords.describe()
		);
		let k = &d.keywords;
		if k.find_todo != k.todo || k.find_done != k.done {
			println!(
				"  find --todo: {} · find --done: {}",
				k.find_todo.join(" "),
				k.find_done.join(" ")
			);
		}
		let mut counts: BTreeMap<String, usize> = BTreeMap::new();
		let mut tags: BTreeMap<String, usize> = BTreeMap::new();
		for h in &d.headings {
			*counts
				.entry(h.keyword.clone().unwrap_or_else(|| "(none)".into()))
				.or_default() += 1;
			for t in &h.tags {
				*tags.entry(t.clone()).or_default() += 1;
			}
		}
		let fmt = |m: &BTreeMap<String, usize>| {
			m.iter()
				.map(|(k, v)| format!("{k}={v}"))
				.collect::<Vec<_>>()
				.join(" ")
		};
		println!("  by keyword: {}", fmt(&counts));
		if !d.filetags.is_empty() {
			println!("  filetags: {}", d.filetags.join(" "));
		}
		println!(
			"  tags: {}",
			if tags.is_empty() {
				"-".into()
			} else {
				fmt(&tags)
			}
		);
		let prot = (0..d.headings.len()).filter(|&i| d.is_protected(i)).count();
		println!(
			"  protected ({}): {prot} heading(s)",
			d.protected_tags.join(",")
		);
	}
	Ok(())
}

fn cmd_outline(docs: &[Doc], depth: Option<usize>, under: Option<String>, ctx: &Ctx) -> Res {
	if under.is_some() && docs.len() > 1 {
		return Err("--under needs a single file".into());
	}
	let prefix = docs.len() > 1;
	let mut out = vec![];
	for d in docs {
		let root = under.as_deref().map(|s| d.resolve(s)).transpose()?;
		if prefix && !ctx.json {
			println!("# {}", d.path.display());
		}
		for i in 0..d.headings.len() {
			let h = &d.headings[i];
			if depth.is_some_and(|m| h.level > m) {
				continue;
			}
			if let Some(r) = root
				&& r != i && !d.is_descendant(i, r)
			{
				continue;
			}
			if ctx.json {
				out.push(heading_out(d, i));
			} else {
				let base = root.map_or(1, |r| d.headings[r].level);
				let indent = "  ".repeat(h.level.saturating_sub(base));
				println!("{indent}{}", heading_line(d, i, false, false));
			}
		}
	}
	if ctx.json { print_json(&out) } else { Ok(()) }
}

fn cmd_find(a: FindArgs, ctx: &Ctx) -> Res {
	let docs = load_all(&a.paths)?;
	if a.under.is_some() && docs.len() > 1 {
		return Err("--under needs a single file".into());
	}
	let date = |s: &Option<String>| {
		s.as_deref()
			.map(|s| DateFilter::parse(s, ctx.today))
			.transpose()
	};
	let (f_sched, f_dl, f_closed) = (date(&a.scheduled)?, date(&a.deadline)?, date(&a.closed)?);
	let f_date = a
		.date
		.as_deref()
		.map(|s| DateRange::parse(s, ctx.today))
		.transpose()?;
	let text_re = a
		.text
		.as_deref()
		.map(|t| {
			let pat = if a.regex {
				t.to_string()
			} else {
				regex::escape(t)
			};
			RegexBuilder::new(&pat)
				.case_insensitive(true)
				.build()
				.map_err(|e| e.to_string())
		})
		.transpose()?;
	let props: Vec<(String, Option<String>)> = a
		.prop
		.iter()
		.map(|p| match p.split_once('=') {
			Some((k, v)) => (k.to_string(), Some(v.to_string())),
			None => (p.clone(), None),
		})
		.collect();

	let prefix = docs.len() > 1;
	let mut hits = vec![];
	for d in &docs {
		for s in &a.status {
			if !s.eq_ignore_ascii_case("none") && d.keywords.canonical(s).is_none() {
				eprintln!(
					"warning: '{s}' is not a keyword in {} ({})",
					d.path.display(),
					d.keywords.describe()
				);
			}
		}
		let root = a.under.as_deref().map(|s| d.resolve(s)).transpose()?;
		for i in 0..d.headings.len() {
			let h = &d.headings[i];
			let kw = h.keyword.as_deref();
			if !a.status.is_empty()
				&& !a.status.iter().any(|s| match kw {
					None => s.eq_ignore_ascii_case("none"),
					Some(k) => s.eq_ignore_ascii_case(k),
				}) {
				continue;
			}
			if a.todo && !kw.is_some_and(|k| d.keywords.find_todo.iter().any(|x| x == k)) {
				continue;
			}
			if a.done && !kw.is_some_and(|k| d.keywords.find_done.iter().any(|x| x == k)) {
				continue;
			}
			if a.level.is_some_and(|l| l != h.level)
				|| a.priority
					.is_some_and(|p| h.priority != Some(p.to_ascii_uppercase()))
			{
				continue;
			}
			if let Some(r) = root
				&& !d.is_descendant(i, r)
			{
				continue;
			}
			if !a.tags.is_empty() || !a.not_tags.is_empty() {
				let all = d.all_tags(i);
				if !a.tags.iter().all(|t| all.contains(t))
					|| a.not_tags.iter().any(|t| all.contains(t))
				{
					continue;
				}
			}
			if f_sched
				.as_ref()
				.is_some_and(|f| !f.matches(h.planning.scheduled.as_ref()))
				|| f_dl
					.as_ref()
					.is_some_and(|f| !f.matches(h.planning.deadline.as_ref()))
				|| f_closed
					.as_ref()
					.is_some_and(|f| !f.matches(h.planning.closed.as_ref()))
			{
				continue;
			}
			if let Some(r) = &f_date {
				let hit = h
					.planning
					.scheduled
					.iter()
					.chain(&h.planning.deadline)
					.any(|t| t.occurs_in(r))
					|| h.timestamps.iter().any(|t| t.active && t.occurs_in(r));
				if !hit {
					continue;
				}
			}
			if !props.iter().all(|(k, v)| match (h.prop(k), v) {
				(Some(_), None) => true,
				(Some(have), Some(want)) => have == want,
				(None, _) => false,
			}) {
				continue;
			}
			if let Some(re) = &text_re {
				let mut hit = re.is_match(&h.title);
				if !hit && a.in_body && !d.is_protected(i) {
					hit = d.lines[h.body_start..h.section_end]
						.iter()
						.any(|l| re.is_match(l));
				}
				if !hit {
					continue;
				}
			}
			hits.push((d, i));
		}
	}
	if let Some(by) = a.sort {
		let kind = match by {
			SortBy::Scheduled => PlanKind::Scheduled,
			SortBy::Deadline => PlanKind::Deadline,
			SortBy::Closed => PlanKind::Closed,
		};
		// stable sort: undated keep file order at the end
		hits.sort_by_key(|(d, i)| {
			let t = d.headings[*i].planning.get(kind);
			(t.is_none(), t.map(|t| (t.date, t.time.clone())))
		});
	}
	let total = hits.len();
	hits.truncate(a.limit);
	if ctx.json {
		return print_json(&json!({
			"total": total,
			"shown": hits.len(),
			"results": hits.iter().map(|(d, i)| heading_out(d, *i)).collect::<Vec<_>>(),
		}));
	}
	for (d, i) in &hits {
		println!("{}", heading_line(d, *i, prefix, true));
	}
	if total > hits.len() {
		println!(
			"… {} more (raise --limit or narrow filters)",
			total - hits.len()
		);
	} else if total == 0 {
		println!("no matches");
	}
	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_show(
	d: &Doc,
	sel: &str,
	body: bool,
	raw: bool,
	subtree: bool,
	max: usize,
	reveal: bool,
	ctx: &Ctx,
) -> Res {
	let i = d.resolve(sel)?;
	let h = &d.headings[i];
	let protected = if subtree && raw {
		(0..d.headings.len()).any(|j| (j == i || d.is_descendant(j, i)) && d.is_protected(j))
	} else {
		d.is_protected(i)
	};
	let hidden = protected && !reveal && (body || raw);
	let br = d.body_range(i);
	let body_lines: Vec<&str> = if body && !hidden {
		d.lines[br.clone()].iter().map(String::as_str).collect()
	} else {
		vec![]
	};
	let raw_lines: Vec<&str> = if raw && !hidden {
		let end = if subtree {
			h.subtree_end
		} else {
			h.section_end
		};
		d.lines[h.line..end].iter().map(String::as_str).collect()
	} else {
		vec![]
	};

	if ctx.json {
		return print_json(&json!({
			"heading": heading_out(d, i),
			"lines": [h.line + 1, h.section_end],
			"subtree_lines": [h.line + 1, h.subtree_end],
			"properties": h.properties.iter().cloned().collect::<BTreeMap<_, _>>(),
			"timestamps": h.timestamps.iter().map(|t| &t.raw).collect::<Vec<_>>(),
			"clock_minutes": h.clock_minutes(),
			"children": h.children.iter().map(|&c| heading_out(d, c)).collect::<Vec<_>>(),
			"body_line_count": br.len(),
			"body": if body && !hidden { Some(body_lines.iter().take(max).collect::<Vec<_>>()) } else { None },
			"raw": if raw && !hidden { Some(raw_lines.iter().take(max).collect::<Vec<_>>()) } else { None },
			"hidden": hidden,
		}));
	}

	println!("{}", heading_line(d, i, false, true));
	println!(
		"lines: {}-{} (subtree to {})",
		h.line + 1,
		h.section_end,
		h.subtree_end
	);
	let inherited: Vec<String> = d
		.all_tags(i)
		.into_iter()
		.filter(|t| !h.tags.contains(t))
		.collect();
	if !inherited.is_empty() {
		println!("inherited tags: {}", inherited.join(" "));
	}
	for (label, t) in [
		("SCHEDULED", &h.planning.scheduled),
		("DEADLINE", &h.planning.deadline),
		("CLOSED", &h.planning.closed),
	] {
		if let Some(t) = t {
			println!("{label}: {}", t.raw);
		}
	}
	for (k, v) in &h.properties {
		println!(":{k}: {v}");
	}
	if !h.clocks.is_empty() {
		let m = h.clock_minutes();
		let running = h.clocks.iter().filter(|c| c.running).count();
		println!(
			"clock: {}h{:02}m in {} entries{}",
			m / 60,
			m % 60,
			h.clocks.len(),
			if running > 0 {
				format!(", {running} running")
			} else {
				String::new()
			}
		);
	}
	let active: Vec<&str> = h
		.timestamps
		.iter()
		.filter(|t| t.active)
		.map(|t| t.raw.as_str())
		.collect();
	if !active.is_empty() {
		println!("timestamps: {}", active.join(" "));
	}
	if protected {
		println!("protected: yes ({})", d.protected_tags.join(","));
	}
	if hidden {
		println!(
			"[content hidden: protected heading; pass --reveal only if the user asked for it]"
		);
	} else if body {
		println!("--- body ({} lines) ---", br.len());
		print_capped(&body_lines, max);
	} else if raw {
		println!("--- raw ---");
		print_capped(&raw_lines, max);
	} else {
		println!("body: {} line(s) (use --body)", br.len());
	}
	if !h.children.is_empty() {
		println!("children ({}):", h.children.len());
		for &c in &h.children {
			println!("  {}", heading_line(d, c, false, false));
		}
	}
	Ok(())
}

fn print_capped(lines: &[&str], max: usize) {
	for l in lines.iter().take(max) {
		println!("{l}");
	}
	if lines.len() > max {
		println!("… {} more line(s) (raise --max-lines)", lines.len() - max);
	}
}

fn cmd_grep(
	docs: &[Doc],
	pattern: &str,
	case_sensitive: bool,
	limit: usize,
	reveal: bool,
	ctx: &Ctx,
) -> Res {
	let insensitive = !case_sensitive && !pattern.chars().any(char::is_uppercase);
	let re = RegexBuilder::new(pattern)
		.case_insensitive(insensitive)
		.build()
		.map_err(|e| e.to_string())?;
	let prefix = docs.len() > 1;
	let mut hits = vec![];
	let mut total = 0;
	let mut skipped = 0;
	for d in docs {
		for (n, l) in d.lines.iter().enumerate() {
			if !re.is_match(l) {
				continue;
			}
			let owner = d.heading_at_line(n);
			let is_heading = owner.is_some_and(|o| d.headings[o].line == n);
			if !reveal && !is_heading && owner.is_some_and(|o| d.is_protected(o)) {
				skipped += 1;
				continue;
			}
			total += 1;
			if hits.len() < limit {
				hits.push((d, n, owner));
			}
		}
	}
	if ctx.json {
		let rows: Vec<_> = hits
			.iter()
			.map(|(d, n, o)| {
				json!({
					"file": d.path.display().to_string(),
					"line": n + 1,
					"id": o.map(|o| d.headings[o].id()),
					"text": truncate(&d.lines[*n], 300),
				})
			})
			.collect();
		return print_json(
			&json!({"total": total, "shown": rows.len(), "skipped_protected": skipped, "results": rows}),
		);
	}
	for (d, n, o) in &hits {
		let file = if prefix {
			format!("{}:", d.path.display())
		} else {
			String::new()
		};
		let id = o.map_or("-".to_string(), |o| d.headings[o].id());
		println!(
			"{file}L{}  [{id}]  {}",
			n + 1,
			truncate(d.lines[*n].trim_end(), 200)
		);
	}
	if total > hits.len() {
		println!("… {} more (raise --limit)", total - hits.len());
	} else if total == 0 {
		println!("no matches");
	}
	if skipped > 0 {
		println!("({skipped} match(es) inside protected headings skipped)");
	}
	Ok(())
}

fn cmd_agenda(docs: &[Doc], range: &str, all: bool, ctx: &Ctx) -> Res {
	let r = DateRange::parse(range, ctx.today)?;
	if r.start.is_none() || r.end.is_none() {
		return Err("agenda needs a closed range, e.g. today..+6d".into());
	}
	let prefix = docs.len() > 1;
	let mut days: BTreeMap<NaiveDate, Vec<String>> = BTreeMap::new();
	let mut overdue = vec![];
	for d in docs {
		for i in 0..d.headings.len() {
			let h = &d.headings[i];
			let done = h.keyword.as_deref().is_some_and(|k| d.keywords.is_done(k));
			if done && !all {
				continue;
			}
			let line = heading_line(d, i, prefix, true);
			let mut seen = vec![];
			let mut add = |kind: &str, t: &ts::Timestamp| {
				for day in t.occurrences(&r, 400) {
					let key = (day, kind.to_string());
					if seen.contains(&key) {
						continue;
					}
					seen.push(key);
					let time = t
						.time
						.as_deref()
						.map(|x| format!("{x} "))
						.unwrap_or_default();
					days.entry(day)
						.or_default()
						.push(format!("  {time}{kind:<5} {line}"));
				}
			};
			if let Some(t) = &h.planning.scheduled {
				add("sched", t);
			}
			if let Some(t) = &h.planning.deadline {
				add("dl", t);
			}
			for t in h.timestamps.iter().filter(|t| t.active) {
				add("ts", t);
			}
			if !done && h.keyword.is_some() && r.contains(ctx.today) {
				for (kind, t) in [
					("sched", &h.planning.scheduled),
					("dl", &h.planning.deadline),
				] {
					if let Some(t) = t
						&& t.date < ctx.today
					{
						let ago = (ctx.today - t.date).num_days();
						overdue.push(format!("  {kind:<5} {ago}d ago  {line}"));
					}
				}
			}
		}
	}
	println!("agenda {}", r.describe());
	if !overdue.is_empty() {
		println!("OVERDUE:");
		overdue.iter().for_each(|l| println!("{l}"));
	}
	for (day, items) in &days {
		let mark = if *day == ctx.today { "  <- today" } else { "" };
		println!(
			"{} {}{mark}",
			day.format("%Y-%m-%d"),
			ts::day_name(*day, false)
		);
		items.iter().for_each(|l| println!("{l}"));
	}
	if overdue.is_empty() && days.is_empty() {
		println!("nothing scheduled");
	}
	Ok(())
}

fn cmd_clock(docs: &[Doc], range: Option<&str>, ctx: &Ctx) -> Res {
	let r = range
		.map(|s| DateRange::parse(s, ctx.today))
		.transpose()?
		.unwrap_or_default();
	let prefix = docs.len() > 1;
	let mut total = 0;
	for d in docs {
		for i in 0..d.headings.len() {
			let cs: Vec<_> = d.headings[i]
				.clocks
				.iter()
				.filter(|c| r.contains(c.start.date))
				.collect();
			if cs.is_empty() {
				continue;
			}
			let m: u32 = cs.iter().filter_map(|c| c.minutes).sum();
			let running = if cs.iter().any(|c| c.running) {
				" (running)"
			} else {
				""
			};
			total += m;
			println!(
				"{:>3}h{:02}m{running}  {}",
				m / 60,
				m % 60,
				heading_line(d, i, prefix, true)
			);
		}
	}
	println!(
		"total {}h{:02}m{}",
		total / 60,
		total % 60,
		if range.is_some() {
			format!(" in {}", r.describe())
		} else {
			String::new()
		}
	);
	Ok(())
}

// ---------- write commands ----------

fn write(file: &Path, sel: &str, ctx: &Ctx, f: impl FnOnce(&mut Doc, usize) -> Res<usize>) -> Res {
	let mut d = Doc::load(file)?;
	let i = resolve_checked(&d, sel, ctx)?;
	let i = f(&mut d, i)?;
	finish_write(&d, Some(i), ctx)
}

fn resolve_checked(d: &Doc, sel: &str, ctx: &Ctx) -> Res<usize> {
	let i = d.resolve(sel)?;
	if let Some(e) = &ctx.expect
		&& !d.headings[i]
			.title
			.to_lowercase()
			.contains(&e.to_lowercase())
	{
		return Err(format!(
			"--expect '{e}' failed: {sel} is now '{}' (ids shift after edits; re-query)",
			heading_line(d, i, false, true)
		));
	}
	Ok(i)
}

fn write_planning(file: &Path, sel: &str, kind: PlanKind, when: &str, ctx: &Ctx) -> Res {
	write(file, sel, ctx, |d, i| {
		let v = if when.eq_ignore_ascii_case("none") {
			None
		} else {
			Some(ts::parse_when(when, ctx.today, d.ru_days)?)
		};
		d.set_planning(i, kind, v)?;
		Ok(i)
	})
}

fn finish_write(d: &Doc, i: Option<usize>, ctx: &Ctx) -> Res {
	if d.changes.is_empty() {
		if ctx.json {
			return print_json(
				&json!({"ok": true, "changed": false, "heading": i.map(|i| heading_out(d, i))}),
			);
		}
		println!("no change");
		return Ok(());
	}
	if !ctx.dry_run {
		d.save()?;
	}
	if ctx.json {
		return print_json(&json!({
			"ok": true,
			"changed": true,
			"saved": !ctx.dry_run,
			"changes": d.changes.iter().map(|c| if c.quiet {
				json!({"line": c.line + 1, "removed_lines": c.removed.len(), "added_lines": c.added.len()})
			} else {
				json!(c)
			}).collect::<Vec<_>>(),
			"heading": i.map(|i| heading_out(d, i)),
		}));
	}
	for c in &d.changes {
		if c.quiet {
			println!(
				"@@ L{}: -{} +{} line(s)",
				c.line + 1,
				c.removed.len(),
				c.added.len()
			);
			continue;
		}
		let shown = c
			.removed
			.iter()
			.map(|l| ('-', l))
			.chain(c.added.iter().map(|l| ('+', l)));
		let n = c.removed.len() + c.added.len();
		println!("@@ L{}", c.line + 1);
		for (sign, l) in shown.take(30) {
			println!("{sign} {l}");
		}
		if n > 30 {
			println!("… {} more line(s)", n - 30);
		}
	}
	if let Some(i) = i {
		println!("=> {}", heading_line(d, i, false, true));
	}
	println!(
		"{}",
		if ctx.dry_run {
			"(dry run, not saved)"
		} else {
			"saved"
		}
	);
	Ok(())
}
