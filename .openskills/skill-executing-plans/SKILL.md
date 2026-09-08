---
description: "Use when partner provides a complete implementation plan to execute in controlled batches with review checkpoints - loads plan, reviews critically, executes tasks in batches, reports for review between batches"
alwaysApply: false
---

# Executing Plans

Use when partner provides a complete implementation plan to execute in controlled batches with review checkpoints - loads plan, reviews critically, executes tasks in batches, reports for review between batches

## Overview

Load plan, review critically, execute tasks in batches, report for review between batches.

**Core principle:** Batch execution with checkpoints for architect review.

**Announce at start:** "I'm using the executing-plans skill to implement this plan."

## The Process

- Read plan file
- Review critically - identify any questions or concerns about the plan
- If concerns: Raise them with your human partner before starting
- If no concerns: Create TodoWrite and proceed
- Default: First 3 tasks
- Mark as in_progress
- Follow each step exactly (plan has bite-sized steps)
- Run verifications as specified
- Mark as completed
- Show what was implemented
- Show verification output
- Say: "Ready for feedback."
- Apply changes if needed
- Execute next batch
- Repeat until complete
- Announce: "I'm using the finishing-a-development-branch skill to complete this work."
- **REQUIRED SUB-SKILL:** Use superpowers:finishing-a-development-branch
- Follow that skill to verify tests, present options, execute choice

## When to Stop and Ask for Help

- STOP executing immediately when:
- Hit a blocker mid-batch (missing dependency, test fails, instruction unclear)
- Plan has critical gaps preventing starting
- You don't understand an instruction
- Verification fails repeatedly
- Ask for clarification rather than guessing.

## When to Revisit Earlier Steps

- Return to Review (Step 1) when:
- Partner updates the plan based on your feedback
- Fundamental approach needs rethinking
- - stop and ask.

## Remember

- Review plan critically first
- Follow plan steps exactly
- Don't skip verifications
- Reference skills when plan says to
- Between batches: just report and wait
- Stop when blocked, don't guess