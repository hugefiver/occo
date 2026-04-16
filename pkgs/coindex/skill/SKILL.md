---
name: coindex
description: "Semantic code search and indexing via GitHub Copilot's External Ingest API. Use this skill whenever the user asks to USE coindex — searching code, indexing a repository, checking if a repo is indexed, managing auth, or running the daemon. Auto-callable for read-only operations (search, status, auth check) — no user confirmation needed. Mutations (index, delete, daemon) require user authorization. When uncertain whether a repository is already indexed, check status first. NOT for developing or modifying the coindex source code itself. Triggers: 'search codebase', 'find code that does X', 'is this repo indexed', 'index this repo', 'coindex', 'semantic search', 'code search', 'update index', 'what's indexed'."
---

# coindex — Semantic Code Index CLI

Build, query, and manage GitHub Copilot semantic code indexes for local git repositories. Computes document hashes and coded symbols locally (WASM), uploads to GitHub for embedding-based search.

## Installation

```shell
cargo install --git https://github.com/hugefiver/occo.git --bin coindex
```

Requires a git repository and GitHub auth (see `auth` command below).

## Permission Model

| Command  | Auto-call | What it does                        |
|----------|-----------|-------------------------------------|
| `status` | ✅ Yes    | List indexed filesets — discovery    |
| `search` | ✅ Yes    | Query the semantic index             |
| `auth`   | ✅ Yes    | Check authentication status          |
| `index`  | ❌ Ask    | Build or update the index (uploads)  |
| `delete` | ❌ Ask    | Remove a fileset from remote         |
| `daemon` | ❌ Ask    | Start background auto-indexer        |

**Read-only** commands (status, search, auth) are safe to call autonomously.
**Mutation** commands (index, delete, daemon) modify remote state — ask the user before running.

## Output Format

Always use `--json` when calling coindex programmatically. The flag goes **before** the subcommand:

```shell
coindex --json <command> [args...]
```

- Suppresses all tracing/log output; only structured JSON on stdout
- `--md` available for markdown-formatted output
- `--json`/`--md` disable interactive auth — commands fail with error if not authenticated

## Discovery: Is This Repo Indexed?

When unsure whether a repo has a semantic index, **check status with the path**:

```shell
coindex --json status /path/to/repo    # check specific repo
coindex --json status .                # check current directory
coindex --json status                  # list all filesets (no path)
```

With a path argument, status resolves the git repo root (works from any subdirectory), derives the fileset name, and returns whether it's indexed:

```json
{"indexed": true, "fileset_name": "myrepo", "repo_root": "/path/to/repo", "status": "completed", "checkpoint": "..."}
```

```json
{"indexed": false, "fileset_name": "myrepo", "repo_root": "/path/to/repo"}
```

Without a path argument, status lists all filesets.

Always check status before attempting search — querying a non-existent fileset fails.

## Commands

### status

```shell
coindex --json status [path]   # with path: check if that repo is indexed
coindex --json status          # without path: list all filesets
```

With `path`: resolves git repo root (even from a subdirectory), finds the matching fileset, returns indexed/not-indexed.
Without `path`: returns all filesets with name, status, and checkpoint.

### search

```shell
coindex --json search "your query" --fileset <NAME> --limit 10
```

- `--fileset` (required): fileset name (usually the repo directory name)
- `--limit`: max results (default 10)

Results contain `location.path`, `location.language`, `distance`, `chunk.text`, `chunk.line_range`.

### auth

```shell
coindex --json auth
```

Returns `{"authenticated": true/false, "source": "coindex"|"opencode"|null}`.

If not authenticated, tell the user to run `coindex auth` interactively — plain mode triggers GitHub device flow.

Token priority: coindex auth file (`~/.local/share/coindex/auth.json`) → OpenCode auth file → interactive device flow (plain only).

### index ⚠️ Confirm first

```shell
coindex --json index [path]                  # full index
coindex --json index [path] --since <SHA>    # incremental
coindex --json index --no-ignore             # include gitignored files
```

Returns `fileset_name`, `head`, `files_indexed`, `files_uploaded`, `elapsed_secs`.

Save the `head` value — pass it as `--since` for future incremental runs. (`api_checkpoint` is internal, ignore it.)

Filtering: gitignored files excluded by default. Hardcoded filters always apply (lockfiles, binaries, dotfiles, `.min.js`, `.d.ts`, `.map`, >1MB).

### delete ⚠️ Confirm first

```shell
coindex --json delete <FILESET_NAME>
```

### daemon ⚠️ Confirm first

```shell
coindex daemon [path] [--interval <secs>] [--no-ignore]
```

Polls git HEAD + working tree at interval. HEAD-only change → incremental; working tree change → full index. First run always full. Runs until killed.

## Error Handling

- `--json` errors: `{"error": "message"}` on stdout, exit code 1
- Upload failures propagate as real errors (not silently swallowed)
- Retries with backoff on 429/5xx
- Failed index → re-run with same `--since` to retry

## Typical Agent Workflow

```
1. coindex --json auth         → check authenticated
2. coindex --json status .     → is current repo indexed?
3. If not indexed → ask user → coindex --json index .
4. coindex --json search "auth middleware" --fileset myrepo --limit 5
```
