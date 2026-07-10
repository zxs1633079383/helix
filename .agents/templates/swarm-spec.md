# Agent Swarm Spec

## Problem

Describe the project workflow problem this swarm solves.

## Generic Skill Boundary

Describe what belongs in `agent-swarm-bootstrap`.

## Project-local Boundary

Describe what belongs in this repo's `.agents/`.

## Architecture

Describe Router, module agents, domain agents, reviewer agents, references, constraints, and runs.

## codebase-memory Integration

Describe how `agent-collect-context` should query codebase-memory and fallback when unavailable.

## Workflow Patterns

List supported workflow patterns and escalation rules.

## Constraint Lifecycle

Describe add, update, deprecate, replace, and remove flows.

## Acceptance Criteria

- Router can form an agent team from a user task.
- `agent-collect-context` can create an impact capsule without project-specific code.
- Scripts validate reports, constraints, references, and runs.
- Directory-level `AGENTS.md` boundaries point to the right agents.
