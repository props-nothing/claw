---
name: Docker Management
description: This skill should be used when the user asks to "manage Docker containers", "build a Docker image", "run docker compose", "debug a container", "check container logs", "clean up Docker", or needs to work with Docker containers, images, compose stacks, or troubleshoot container issues.
version: 1.0.0
tags: [docker, containers, devops, deployment]
author: Claw Team
---

# Docker Management

## Overview

Procedural guide for managing Docker containers, images, and compose stacks. Includes troubleshooting patterns for common container issues.

## Prerequisites

- Docker must be installed — verify with `shell_exec` running `which docker`
- Docker daemon must be running — `docker info` should succeed
- For compose: `docker compose version` (v2) or `docker-compose --version` (v1)

## Container Operations

### List & Inspect

```bash
# List running containers
docker ps

# List all containers (including stopped)
docker ps -a

# Inspect a container
docker inspect <container>

# View container logs
docker logs <container> --tail 100

# Follow logs in real-time (use process_start for this)
docker logs -f <container>
```

### Lifecycle

```bash
# Start/stop/restart
docker start <container>
docker stop <container>
docker restart <container>

# Remove a container
docker rm <container>

# Run a new container
docker run -d --name <name> -p <host>:<container> <image>

# Execute a command in a running container
docker exec -it <container> bash
docker exec <container> <command>
```

## Image Operations

```bash
# List images
docker images

# Build an image
docker build -t <name>:<tag> .

# Pull an image
docker pull <image>:<tag>

# Remove an image
docker rmi <image>

# Tag and push
docker tag <image> <registry>/<image>:<tag>
docker push <registry>/<image>:<tag>
```

## Docker Compose

### Common Commands

```bash
# Start all services
docker compose up -d

# Stop all services
docker compose down

# View status
docker compose ps

# View logs
docker compose logs <service>

# Rebuild and restart
docker compose up -d --build

# Scale a service
docker compose up -d --scale <service>=3
```

### Writing docker-compose.yml

When creating compose files:
- Use version 3.x syntax
- Define services with proper networking
- Use named volumes for persistence
- Set restart policies (`unless-stopped` or `always`)
- Use environment variables for configuration
- Add healthchecks for critical services

## Troubleshooting

### Container won't start

1. Check logs: `docker logs <container>`
2. Verify image exists: `docker images | grep <image>`
3. Check port conflicts: `docker ps` and `lsof -i :<port>`
4. Try running interactively: `docker run -it <image> bash`

### Out of disk space

```bash
# Check disk usage
docker system df

# Clean up unused resources
docker system prune -f

# Remove unused volumes
docker volume prune -f

# Remove unused images
docker image prune -a -f
```

### Networking issues

```bash
# List networks
docker network ls

# Inspect a network
docker network inspect <network>

# Test connectivity between containers
docker exec <container1> ping <container2>
```

## Important Notes

- Check `docker ps` before making changes to understand current state
- Use `--tail 100` with logs to avoid overwhelming output
- For long-running log watching, use `process_start` instead of `shell_exec`
- Back up volumes before removing containers with persistent data
- Prefer `docker compose` (v2) over `docker-compose` (v1) when available
