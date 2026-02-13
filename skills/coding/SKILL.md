---
name: Coding
description: This skill should be used when the user asks to "write code", "debug this", "refactor code", "review my code", "fix this bug", "add tests", "create a function", or needs full-stack development assistance including writing, debugging, refactoring, and testing code in any language.
version: 1.0.0
tags: [development, coding, debugging, refactoring]
author: Claw Team
---

# Coding

## Overview

Procedural guide for full-stack development — analyzing project structure, writing production-quality code, debugging, refactoring, and running tests across any programming language.

## Workflow

### 1. Understand the Project

Before writing any code:
- Explore the project structure with `file_list` on the root directory
- Read key config files: `package.json`, `Cargo.toml`, `pyproject.toml`, etc.
- Identify the tech stack and conventions already in use
- Check for existing patterns in similar files

### 2. Explore Before Editing

- Use `file_find` to locate relevant files by name
- Use `file_grep` to search for related code, imports, and usages
- Read existing code to understand patterns, naming conventions, and architecture
- Check for tests to understand expected behavior

### 3. Write Code

- Use `file_write` for new files
- Use `file_edit` for modifying existing files (find-and-replace semantics)
- For complex multi-file changes, use `apply_patch` with unified diff format
- Write production-quality code — not placeholder stubs

### 4. Verify Changes

After writing code:
- Run the build with `shell_exec` using the appropriate build command
- Run tests with `shell_exec` using the test runner
- Fix any errors — read error messages carefully and iterate
- For web projects, use `browser_navigate` to verify visually

## Language-Specific Patterns

### JavaScript/TypeScript

```bash
npm install          # Install dependencies
npm run dev          # Dev server (use terminal_open + terminal_run)
npm run build        # Build
npm test             # Run tests
```

### Rust

```bash
cargo build          # Build
cargo test           # Run tests
cargo check          # Check without building
cargo fmt            # Format
cargo clippy         # Lint
```

### Python

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
python -m pytest     # Run tests
python -m mypy .     # Type checking
```

### Go

```bash
go build ./...       # Build
go test ./...        # Test
go run .             # Run
```

## Debugging Strategies

1. **Read the error message** — it usually tells exactly what went wrong
2. **Add logging** — use print/console.log/tracing::debug to trace execution
3. **Check types and signatures** — mismatched types are a common source of errors
4. **Binary search** — if a change broke something, narrow down which part
5. **Check imports** — missing or wrong imports cause confusing errors
6. **Read the docs** — use `http_fetch` to check API documentation

## Code Review Checklist

When reviewing code:
- [ ] Proper error handling
- [ ] Edge cases covered
- [ ] Readable and well-named
- [ ] No security issues (SQL injection, XSS, etc.)
- [ ] No unnecessary duplication
- [ ] Tests present
- [ ] Follows existing project patterns

## Important Notes

- Explore the project structure before making changes
- Match the existing code style — do not introduce new conventions without reason
- Run builds/tests after changes to catch issues early
- For interactive/long-running processes (dev servers, watchers), use `terminal_open` + `terminal_run` instead of `shell_exec`
- Write complete implementations, not TODOs or placeholders
