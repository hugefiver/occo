---
name: coindex
description: "Use this skill to build, update, search, and manage GitHub Copilot semantic code indexes via the coindex CLI. Triggers: 'index repo', 'semantic search', 'code index', 'coindex', 'update index', 'search codebase'. Covers: full/incremental indexing, search, fileset management, daemon mode for auto-indexing on changes."
---

# coindex — Copilot Semantic Code Index CLI

## What It Does

`coindex` builds and queries GitHub Copilot's semantic code index for local repositories via the External Ingest API. It computes document hashes, geo-filters, and coded symbols locally (WASM), then uploads to GitHub for embedding-based search.

## Installation

```shell
cargo install --git https://github.com/hugefiver/occo.git --bin coindex
```

## Prerequisites

- Git repository (coindex operates on git-tracked files)
- Auth: coindex checks its own token at `~/.local/share/coindex/auth.json` first, then falls back to OpenCode's `auth.json` (`occo.refresh` field). If neither exists, interactive GitHub device flow is available in plain mode.

## Output Format Flags (Global)

All commands support output format flags. **Agents MUST use `--json` for structured output.**

```shell
# JSON output (machine-readable, suppresses all logs)
coindex --json <command> [args...]

# Markdown output (human-readable structured, suppresses all logs)
coindex --md <command> [args...]

# Plain output (default, tracing logs to stderr, interactive auth allowed)
coindex <command> [args...]
```

**Rules:**
- `--json` and `--md` are **global flags** — they go BEFORE the subcommand
- When active, all tracing/log output is suppressed; only structured output goes to stdout
- Errors are formatted in the selected format (see Error Handling)
- `--json` takes precedence if both `--json` and `--md` are specified
- `--json`/`--md` disables interactive auth — if not authenticated, commands fail with an error instead of prompting

## Commands

### index — Build or update the semantic index

```shell
# Full index of current directory
coindex index

# Full index of specific path
coindex index /path/to/repo

# Incremental index: only files changed since a previous git commit
coindex index --since <GIT_COMMIT_SHA>

# Include .gitignore'd files (not recommended unless user requests)
coindex index --no-ignore
```

**Parameters:**
- `path` (positional, default `.`): path to the repository
- `--since <SHA>` (alias `--checkpoint`): git commit ref for incremental mode. Use the `head` value from a previous index run.
- `--no-ignore`: include files matching `.gitignore` rules

**Behavior:**
- Without `--since`: full index of all git-tracked files
- With `--since`: indexes files changed between that commit and current HEAD (additions, modifications, renames). Deletions are detected and handled — even delete-only changes update the remote index.
- Files matching `.gitignore` rules are excluded by default (even if tracked). Use `--no-ignore` to override.
- Hardcoded filters always apply regardless of `--no-ignore`: lockfiles, binary extensions, dotfiles, `.min.js`/`.min.css`/`.map`/`.d.ts`, files >1MB, files with >50% non-ASCII bytes, files with avg line >500 chars.

**JSON output:**
```json
{
  "fileset_name": "myrepo",
  "head": "abc123def456...",
  "api_checkpoint": "base64...",
  "files_indexed": 42,
  "files_uploaded": 10,
  "elapsed_secs": 3.5
}
```

**Key:** The `head` field is the git commit SHA to pass back as `--since` for the next incremental run. The `api_checkpoint` is an internal API value — do NOT use it as `--since`.

### search — Query the semantic index

```shell
coindex search "your natural language query" --fileset <NAME> --limit 10
```

- `--fileset` (required): name of the indexed fileset (usually the repo directory name)
- `--limit`: max results (default 10)

**JSON output:** Full `SearchResponse` with `results` array containing `location.path`, `location.language`, `distance`, `chunk.text`, `chunk.line_range`.

### status — List indexed filesets

```shell
coindex status
```

Returns all filesets with their names, status, and checkpoint values.

### delete — Remove a fileset

```shell
coindex delete <FILESET_NAME>
```

Removes a fileset from the remote index.

### auth — Authentication status and login

```shell
# Plain mode: shows status, triggers device flow if needed
coindex auth

# JSON mode: shows status only, never triggers device flow
coindex --json auth
```

**JSON output:**
```json
{"authenticated": true, "source": "coindex"}
```

`source` is `"coindex"` or `"opencode"` depending on which token file was found. When not authenticated: `{"authenticated": false, "source": null}`.

**Token priority:**
1. `~/.local/share/coindex/auth.json` (coindex-specific)
2. OpenCode `auth.json` (`occo.refresh` field)
3. Interactive GitHub device flow (plain mode only, saves to coindex auth file)

### daemon — Auto-index on changes

```shell
# Watch current dir, poll every 30s
coindex daemon

# Custom path and interval
coindex daemon /path/to/repo --interval 60

# Include gitignored files
coindex daemon --no-ignore
```

**Behavior:**
- Polls both git HEAD and working tree status at the specified interval
- HEAD change only (new commits, pull, merge) → incremental index using previous HEAD as `--since`
- Working tree change (unstaged/staged edits, new files, even combined with HEAD change) → full index
- First run always does a full index
- Non-interactive token acquisition when started with `--json`/`--md` (retries silently if token unavailable — tracing is suppressed in non-plain mode)
- Errors are logged but don't stop the daemon; it retries on the next cycle
- Runs indefinitely until killed (Ctrl+C / SIGINT)

## Agent Usage Patterns

**Always use `--json` for programmatic access.**

### First-time full index

```shell
coindex --json index /path/to/repo
```

Parse JSON → save `head` for future incremental updates.

### Incremental update

```shell
coindex --json index /path/to/repo --since "HEAD_FROM_PREVIOUS_RUN"
```

### Search

```shell
coindex --json search "authentication middleware" --fileset myrepo --limit 5
```

### Check status

```shell
coindex --json status
```

### Check auth (non-interactive)

```shell
coindex --json auth
```

If `authenticated` is false, instruct the user to run `coindex auth` manually.

### Background auto-indexing

```shell
coindex daemon /path/to/repo --interval 60
```

## Error Handling

- `--json` errors: `{"error": "message"}` on stdout, exit code 1
- `--md` errors: `**Error**: message` on stdout, exit code 1
- Plain mode errors: printed to stderr via tracing
- Upload failures are **not** silently swallowed — they propagate as errors and fail the index run
- Network requests include retry with exponential backoff for 429 and 5xx responses
- If indexing fails, re-run with the same `--since` value to retry from that point
- The daemon logs errors per cycle but continues running

## Environment Variables

- `RUST_LOG`: controls tracing verbosity (plain mode only — suppressed with `--json`/`--md`). Default: `info`. Examples: `debug`, `warn`, `coindex=debug`.
