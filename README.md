# ROrgParser - Rust Org-Mode File Parser

A command-line tool for parsing Emacs org-mode files written in Rust. This parser extracts the hierarchical structure of org-mode files, including headings, statuses, tags, and content.

## Features

- **Hierarchical Parsing**: Correctly parses nested org-mode headings (*, **, ***, etc.)
- **Status Detection**: Recognizes task statuses like TODO, DONE, IN-PROGRESS, etc.
- **Tag Support**: Extracts org-mode tags (text between colons at the end of headings)
- **Content Extraction**: Captures all text content between headings
- **Multiple Output Formats**: Supports both human-readable text and JSON output
- **Verbose Mode**: Provides detailed parsing information

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

# Get help
rorg --help
```

### Command Line Options

- `<file>`: The org-mode file to parse (required)
- `-v, --verbose`: Enable verbose output showing file statistics
- `-f, --format <format>`: Output format, either `text` (default) or `json`
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

### Content
All text between headings is captured as content for the preceding heading.

## Example

### Input (example.org)
```org
#+title: My Project
#+author: John Doe

* TODO Project Setup :urgent:important:
This is the main project setup task.
It needs to be completed first.

** DONE Initialize repository :git:
CLOSED: [2024-01-01 Mon 10:00]
The repository has been created.

** IN-PROGRESS Setup development environment :dev:docker:
Still working on the Docker configuration.

* DONE Documentation :docs:
CLOSED: [2024-01-01 Mon 15:00]
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
Level: 1
Status: TODO
Title: Project Setup
Labels: ["urgent", "important"]
Content:
  This is the main project setup task.
  It needs to be completed first.
Children:
  Level: 2
  Status: DONE
  Title: Initialize repository
  Labels: ["git"]
  Content:
    CLOSED: [2024-01-01 Mon 10:00]
    The repository has been created.

  Level: 2
  Status: IN-PROGRESS
  Title: Setup development environment
  Labels: ["dev", "docker"]
  Content:
    Still working on the Docker configuration.

Level: 1
Status: DONE
Title: Documentation
Labels: ["docs"]
Content:
  CLOSED: [2024-01-01 Mon 15:00]
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
        "content": "CLOSED: [2024-01-01 Mon 10:00]\nThe repository has been created.\n",
        "children": []
      }
    ]
  }
]
```

## Data Structure

The parser creates a hierarchical structure where each org heading becomes an `OrgNote` with the following fields:

- `level`: Number of asterisks (heading depth)
- `status`: Optional status keyword (TODO, DONE, etc.)
- `title`: The heading text without status and tags
- `labels`: Array of tags extracted from the heading
- `content`: Raw text content until the next heading
- `children`: Array of child `OrgNote` objects

## Limitations

- Currently focuses on basic org-mode structure (headings, status, tags, content)
- Does not parse org-mode specific elements like tables, code blocks, or properties
- Tags must be at the end of the heading line in the format `:tag1:tag2:`
- Status keywords must be uppercase and immediately follow the asterisks

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the MIT License - see the LICENSE file for details.