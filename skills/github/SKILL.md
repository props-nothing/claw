---
name: GitHub Management
description: This skill should be used when the user asks to "create a repo", "open a pull request", "review a PR", "manage GitHub issues", "create a release", "fork a repo", "clone a repository", or needs to interact with GitHub for repository, PR, issue, or release management.
version: 1.0.0
tags: [git, github, development, code-review]
author: Claw Team
---

# GitHub Management

## Overview

Procedural guide for managing GitHub repositories, pull requests, issues, and releases using the GitHub CLI (`gh`) and Git.

## Prerequisites

- GitHub CLI (`gh`) should be installed — verify with `shell_exec` running `which gh`
- If not installed: `brew install gh` (macOS) or see https://cli.github.com
- Authenticate with `gh auth login` if needed
- Alternatively, use the GitHub API with a personal access token

## Repository Operations

```bash
# Create a new repo
gh repo create <name> --public/--private --description "desc"

# Clone a repo
gh repo clone <owner/repo>

# View repo info
gh repo view <owner/repo>

# Fork a repo
gh repo fork <owner/repo>
```

## Pull Requests

```bash
# Create a PR
gh pr create --title "Title" --body "Description" --base main

# List open PRs
gh pr list

# View a specific PR
gh pr view <number>

# Review a PR
gh pr review <number> --approve/--request-changes --body "Review comments"

# Merge a PR
gh pr merge <number> --squash/--merge/--rebase
```

## Issues

```bash
# Create an issue
gh issue create --title "Title" --body "Description" --label "bug"

# List issues
gh issue list

# Close an issue
gh issue close <number>
```

## Releases

```bash
# Create a release
gh release create <tag> --title "Release Title" --notes "Release notes"

# List releases
gh release list

# Upload assets to a release
gh release upload <tag> <file>
```

## Code Review Workflow

1. Fetch the PR diff: `gh pr diff <number>`
2. Read the changed files using `file_read`
3. Analyze the code for:
   - Bug risks and logic errors
   - Security vulnerabilities
   - Performance issues
   - Code style and best practices
   - Missing tests
4. Post review comments: `gh pr review <number> --body "..."`

## Working with Git Directly

For local git operations, use `shell_exec`:

```bash
git status
git add -A
git commit -m "message"
git push origin branch-name
git log --oneline -20
git diff HEAD~1
```

## Important Notes

- Check the current directory and git status before making changes
- Use `gh auth status` to verify authentication
- For private repos, ensure proper access tokens are configured
- Store repository URLs and common commands in memory for quick access
