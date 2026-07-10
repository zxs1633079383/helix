# Agent Swarm Implementation Plan

> Required workflow: spec first, plan second, TDD for behavior changes.

## Goal

Describe the working swarm capability this plan builds.

## Global Constraints

- Do not hard-code project-specific facts into the Skill.
- Keep project facts inside repo-local `.agents/`.
- Write a failing test before changing deterministic script behavior.
- Use codebase-memory first for impact discovery when no explicit paths are supplied.
- Fallback to static keyword/path matching when codebase-memory is unavailable.

## Task Template

### Task N: Behavior Name

**Files:**
- Test: `exact/test/path`
- Modify: `exact/source/path`

**Interfaces:**
- Consumes: exact input shape.
- Produces: exact output shape.

- [ ] Step 1: Write the failing test.
- [ ] Step 2: Run the test and verify RED.
- [ ] Step 3: Implement the minimum code.
- [ ] Step 4: Run the test and verify GREEN.
- [ ] Step 5: Refactor only after GREEN.
- [ ] Step 6: Run validation commands.
