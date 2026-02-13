---
name: Web Research
description: This skill should be used when the user asks to "research a topic", "find information about", "compare products", "look up documentation", "search the web for", or needs to search the web, fetch pages, extract information, and synthesize findings on any topic.
version: 1.0.0
tags: [research, web, search, information]
author: Claw Team
---

# Web Research

## Overview

Procedural guide for researching topics on the web — crafting effective searches, fetching and reading pages, and synthesizing findings into clear summaries.

## Research Workflow

### 1. Search

Use `web_search` with well-crafted queries:
- Be specific: "Python asyncio best practices 2025" not "Python tips"
- Use multiple searches with different angles
- Try both broad and narrow queries

### 2. Fetch & Read

For each promising result:
- Use `http_fetch` to get the full page content
- Extract the relevant information
- Note the source URL for attribution

### 3. Synthesize

After gathering information:
- Combine findings from multiple sources
- Identify areas of consensus and disagreement
- Present a clear, organized summary
- Cite sources

## Search Strategies

### General research

1. Start with a broad search to understand the landscape
2. Narrow down with specific queries based on initial findings
3. Look for authoritative sources (official docs, academic papers, reputable sites)

### Technical research

1. Search for official documentation first
2. Look for recent blog posts and tutorials (within last 1-2 years)
3. Check GitHub for example implementations
4. Look at Stack Overflow for common pitfalls

### Product/Service comparison

1. Search for "[product A] vs [product B]"
2. Look for independent review sites
3. Check pricing pages directly
4. Look for user reviews and experiences

## Tips for Better Results

- Use `http_fetch` on promising URLs from search results to get full details
- For paywalled or JavaScript-heavy sites, try `browser_navigate` + `browser_screenshot`
- Save important findings to memory with `memory_store` for future reference
- When comparing things, create a structured comparison (table format works well)

## Important Notes

- Always provide sources for findings
- Distinguish between facts, opinions, and speculation
- Note when information might be outdated
- For time-sensitive topics, prioritize recent sources
- If a page fails to load, try an alternative source
