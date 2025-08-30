# ROrgParser - Rust Org-Mode File Parser

A command-line & tui tool for parsing Emacs org-mode files written in Rust. This parser extracts the hierarchical structure of org-mode files, including headings, statuses, tags, and content.

> after several tries vibecoding this I realize it would probably better to move to pure emacs with only org mode

## Features

- **Hierarchical Parsing**: Correctly parses nested org-mode headings (*, **, ***, etc.)
- **Status Detection**: Recognizes task statuses like TODO, DONE, IN-PROGRESS, etc.
- **Tag Support**: Extracts org-mode tags (text between colons at the end of headings)
- **Content Extraction**: Captures all text content between headings
- **Time Tracking**: Parses LOGBOOK entries with CLOCK timestamps and duration calculations
- **Planning Support**: Extracts SCHEDULED, DEADLINE, and CLOSED timestamps
- **Time Statistics**: Provides summary statistics for tracked time and task completion
- **Multiple Output Formats**: Supports both human-readable text and JSON output
- **Verbose Mode**: Provides detailed parsing information

## Known issues
- Editing clock time in note metadata is not working

## Installation

### From Source

1. Clone the repository:
```bash
git clone <repository-url>
cd rorg
```

2. Build with Cargo:
```bash
cargo build --release
```

3. The binary will be available at `target/release/rorg`

## Usage

### Basic Usage

```bash
# Parse an org file with default text output
rorg myfile.org

# Parse with verbose information
rorg --verbose myfile.org

# Output as JSON
rorg --format json myfile.org

# Show time tracking summary
rorg --summary myfile.org

# Combine options
rorg --verbose --summary --format json myfile.org

# Get help
rorg --help
```

### Command Line Options

- `<file>`: The org-mode file to parse (required)
- `-v, --verbose`: Enable verbose output showing file statistics
- `-f, --format <format>`: Output format, either `text` (default) or `json`
- `-s, --summary`: Show time tracking summary statistics
- `-h, --help`: Show help information
- `-V, --version`: Show version information

## Org-Mode Structure Support

The parser recognizes the following org-mode elements:

### Headings
- `*` Level 1 heading
- `**` Level 2 heading
- `***` Level 3 heading
- And so on...

### Status Keywords
Any uppercase word immediately following the asterisks is treated as a status:
- `* TODO My task`
- `* DONE Completed task`
- `* IN-PROGRESS Active task`
- `* CANCELLED Cancelled task`

### Tags
Tags are extracted from text between colons at the end of headings:
- `* TODO My task :urgent:important:`
- `** DONE Subtask :work:project:`

### Planning Keywords
Planning information is extracted from content:
- `SCHEDULED: <2024-01-20 Sat 09:00>`
- `DEADLINE: <2024-01-31 Wed>`
- `CLOSED: [2024-01-15 Mon 17:30]`

### Time Tracking (LOGBOOK)
LOGBOOK blocks with CLOCK entries are parsed:
```org
:LOGBOOK:
CLOCK: [2024-01-15 Mon 09:00]--[2024-01-15 Mon 12:00] =>  3:00
CLOCK: [2024-01-16 Tue 14:00]--[2024-01-16 Tue 17:30] =>  3:30
:END:
```

### Content
All text between headings is captured as content for the preceding heading.

## Example

### Input (example.org)
```org
#+title: My Project
#+author: John Doe

* TODO Project Setup :urgent:important:
SCHEDULED: <2024-01-20 Sat 09:00>
DEADLINE: <2024-01-31 Wed>

This is the main project setup task.
It needs to be completed first.

** DONE Initialize repository :git:
CLOSED: [2024-01-01 Mon 10:00]
:LOGBOOK:
CLOCK: [2024-01-01 Mon 09:00]--[2024-01-01 Mon 10:00] =>  1:00
:END:

The repository has been created.

** IN-PROGRESS Setup development environment :dev:docker:
:LOGBOOK:
CLOCK: [2024-01-02 Tue 14:00]--[2024-01-02 Tue 17:30] =>  3:30
CLOCK: [2024-01-03 Wed 10:00]
:END:

Still working on the Docker configuration.

* DONE Documentation :docs:
SCHEDULED: <2024-01-10 Wed 14:00>
CLOSED: [2024-01-15 Mon 16:45]
:LOGBOOK:
CLOCK: [2024-01-12 Fri 13:00]--[2024-01-12 Fri 18:00] =>  5:00
CLOCK: [2024-01-15 Mon 14:00]--[2024-01-15 Mon 16:45] =>  2:45
:END:

All documentation is complete.

** Research phase
Initial research was thorough.

** Writing phase
Documentation written and reviewed.
```

### Text Output
```bash
$ rorg example.org
```

```
Parsed org-mode structure:
========================
Time Tracking Summary:
=====================
Total tracked time: 12h 15m
Completed tasks: 2
Active tasks: 1
Scheduled tasks: 2

Level: 1
Status: TODO
Title: Project Setup
Labels: ["urgent", "important"]
Planning:
  Scheduled: <2024-01-20 Sat 09:00> (2024-01-20 09:00)
  Deadline: <2024-01-31 Wed> (2024-01-31)
Content:
  This is the main project setup task.
  It needs to be completed first.
Children:
  Level: 2
  Status: DONE
  Title: Initialize repository
  Labels: ["git"]
  Planning:
    Closed: [2024-01-01 Mon 10:00] (2024-01-01 10:00)
  Time Tracking: (total: 1h 0m)
    Clock: 2024-01-01 09:00 => 1:00 (60 minutes)
  Content:
    The repository has been created.

  Level: 2
  Status: IN-PROGRESS
  Title: Setup development environment
  Labels: ["dev", "docker"]
  Time Tracking: (total: 3h 30m)
    Clock: 2024-01-02 14:00 => 3:30 (210 minutes)
    Clock: 2024-01-03 10:00 (running)
  Content:
    Still working on the Docker configuration.

Level: 1
Status: DONE
Title: Documentation
Labels: ["docs"]
Planning:
  Scheduled: <2024-01-10 Wed 14:00> (2024-01-10 14:00)
  Closed: [2024-01-15 Mon 16:45] (2024-01-15 16:45)
Time Tracking: (total: 7h 45m)
  Clock: 2024-01-12 13:00 => 5:00 (300 minutes)
  Clock: 2024-01-15 14:00 => 2:45 (165 minutes)
Content:
  All documentation is complete.
Children:
  Level: 2
  Title: Research phase
  Content:
    Initial research was thorough.

  Level: 2
  Title: Writing phase
  Content:
    Documentation written and reviewed.
```

### Summary Output
```bash
$ rorg --summary example.org
```

```
Time Tracking Summary:
=====================
Total tracked time: 12h 15m
Completed tasks: 2
Active tasks: 1
Scheduled tasks: 2

Parsed org-mode structure:
========================
[... full structure follows ...]
```

### JSON Output
```bash
$ rorg --format json example.org
```

```json
[
  {
    "level": 1,
    "status": "TODO",
    "title": "Project Setup",
    "labels": ["urgent", "important"],
    "content": "This is the main project setup task.\nIt needs to be completed first.\n",
    "children": [
      {
        "level": 2,
        "status": "DONE",
        "title": "Initialize repository",
        "labels": ["git"],
        "content": "The repository has been created.\n",
        "children": [],
        "planning": {
          "scheduled": null,
          "deadline": null,
          "closed": {
            "year": 2024,
            "month": 1,
            "day": 1,
            "hour": 10,
            "minute": 0,
            "day_name": "Mon",
            "raw": "[2024-01-01 Mon 10:00]"
          }
        },
        "logbook": {
          "clock_entries": [
            {
              "start": {
                "year": 2024,
                "month": 1,
                "day": 1,
                "hour": 9,
                "minute": 0,
                "day_name": "Mon",
                "raw": "[2024-01-01 Mon 09:00]"
              },
              "end": {
                "year": 2024,
                "month": 1,
                "day": 1,
                "hour": 10,
                "minute": 0,
                "day_name": "Mon",
                "raw": "[2024-01-01 Mon 10:00]"
              },
              "duration": "1:00",
              "raw": "CLOCK: [2024-01-01 Mon 09:00]--[2024-01-01 Mon 10:00] =>  1:00"
            }
          ],
          "raw_content": ["CLOCK: [2024-01-01 Mon 09:00]--[2024-01-01 Mon 10:00] =>  1:00"]
        }
      }
    ],
    "planning": {
      "scheduled": {
        "year": 2024,
        "month": 1,
        "day": 20,
        "hour": 9,
        "minute": 0,
        "day_name": "Sat",
        "raw": "<2024-01-20 Sat 09:00>"
      },
      "deadline": {
        "year": 2024,
        "month": 1,
        "day": 31,
        "hour": null,
        "minute": null,
        "day_name": "Wed",
        "raw": "<2024-01-31 Wed>"
      },
      "closed": null
    },
    "logbook": null
  }
]
```

## Data Structure

The parser creates a hierarchical structure where each org heading becomes an `OrgNote` with the following fields:

- `level`: Number of asterisks (heading depth)
- `status`: Optional status keyword (TODO, DONE, etc.)
- `title`: The heading text without status and tags
- `labels`: Array of tags extracted from the heading
- `content`: Raw text content until the next heading (excludes LOGBOOK and planning)
- `children`: Array of child `OrgNote` objects
- `planning`: Optional planning information (SCHEDULED, DEADLINE, CLOSED timestamps)
- `logbook`: Optional time tracking information (CLOCK entries with durations)

## Time Tracking Features

### Supported Timestamp Formats
- `[2024-01-01 Mon 10:00]` - Closed timestamps (square brackets)
- `<2024-01-20 Sat 09:00>` - Active timestamps (angle brackets)
- Supports both date-only and date-time formats
- Handles various day name formats (Mon, Monday, Пн, etc.)

### LOGBOOK Processing
- Automatically extracts CLOCK entries from `:LOGBOOK:` blocks
- Calculates total time from duration entries
- Supports both completed and running clock entries
- Removes LOGBOOK content from the main content text

### Planning Keywords
- `SCHEDULED:` - When a task should be started
- `DEADLINE:` - When a task must be completed
- `CLOSED:` - When a task was actually completed
- All planning information is extracted from content

### Time Statistics
The `--summary` option provides:
- Total tracked time across all tasks
- Count of completed, active, and scheduled tasks
- Warning about overdue tasks

## Limitations

- Does not parse org-mode specific elements like tables, code blocks, or properties (except time tracking)
- Tags must be at the end of the heading line in the format `:tag1:tag2:`
- Status keywords must be uppercase and immediately follow the asterisks
- CLOCK duration calculations rely on the duration field in the org format (` => HH:MM`)
- Overdue detection uses simple date comparison (may need adjustment for your timezone)

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the MIT License - see the LICENSE file for details.
