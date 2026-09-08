---
# GitHub issue tracker (required).
# Set GITHUB_TOKEN in the environment; workflow resolves $GITHUB_TOKEN at runtime.
# Token: for orchestrator-only, Issues: Read-only (or classic public_repo/repo). For PR-driven workflow the agent needs write: Issues, Pull requests, Contents. See docs/SPEC/10-github-tracker.md.
# Addendum 1 (docs/SPEC_ADDENDUM_1.md): include_labels / exclude_labels filter candidates; claim_label, pr_open_label, pr_base_branch are required; worktree.root is required.
tracker:
  repo: "Industrial/rust-symphony"
  api_key: "$GITHUB_TOKEN"
  claim_label: "symphony-claimed"
  pr_open_label: "pr-open"
  pr_base_branch: "main"
  active_states: ["open"]
  terminal_states: ["closed"]
  include_labels: ["symphony", "bot"]
  exclude_labels: ["symphony-claimed", "wip"]
  mention_handle: "symphony"

# Command to run the coding agent in each git worktree (required).
# Change this to use a different agent; it is run with cwd = per-issue git worktree.
# type: "codex" (default) | "acp" | "cli"
# codex = Codex-style protocol (thread/start, turn/start, turn/completed).
# acp = Cursor ACP (agent acp). Command: "agent acp".
# cli = Cursor non-interactive: prompt as argument, parse stream-json. Use when "agent acp" is not available (e.g. NixOS).
# Optional: set SYMPHONY_EXIT_ON_WORKER_FAILURE=1 to exit with code 1 on first worker failure.
# Debug: RUST_LOG=debug to see agent_direction=send|recv and agent_line.
runner:
  type: cli
  # command: "/run/current-system/sw/bin/cursor-agent --force --approve-mcps --model auto --force --workspace . --print --output-format stream-json --stream-partial-output"
  command: "/run/current-system/sw/bin/cursor-agent --force --approve-mcps --model auto --force --workspace . --print --output-format text"
  turn_timeout_ms: 3600000
  read_timeout_ms: 60000
  stall_timeout_ms: 300000

# How often to poll the tracker (default 30_000 ms).
polling:
  interval_ms: 60000

# worktree.root (required): root directory for per-issue git worktrees. Supports $VAR and ~.
# main_repo_path (required): path to the main git repository; workers get a git worktree and branch per issue.
worktree:
  root: "./.symphony_worktrees"
  main_repo_path: "."

# Optional: agent concurrency and retry.
agent:
  max_concurrent_agents: 3
  max_turns: 20
  max_retry_backoff_ms: 300000

# Addendum 2 (docs/SPEC_ADDENDUM_2.md): fix_pr re-dispatches the agent when the PR has failing checks or when someone mentions the configured handle (e.g. @symphony).
fix_pr: true
---

# Prompt template (how the agent receives the ticket)

The text below is the prompt sent to the agent for each issue. Edit it to change how the ticket is presented.
Liquid variables: `issue` (title, identifier, state, description, url, labels), optional `attempt` (retry number), optional `workflow` (when set: `workflow.pr_base_branch` — base branch for worker branches and PR target, e.g. main or develop).
See https://shopify.github.io/liquid/ for syntax.

**Tracker and completion:** The runner only **reads** the tracker (no write access). When the agent is done, it should **close the issue** (or move it to a terminal state) using its own tools (e.g. GitHub CLI, API, or a comment for a human to close). Once the issue is closed, the runner will stop re-dispatching it on the next poll.

**Claim label (Addendum 1):** Add the claim label `symphony-claimed` to this issue as your first step (e.g. `gh issue edit … --add-label symphony-claimed`). That prevents other workers from picking the same issue and survives restarts.

**PR-driven handoff (Addendum 1):** Worker branches MUST be based off the configured base branch (e.g. fetch and checkout `main` or the branch in `workflow.pr_base_branch`, then create `symphony/issue-<number>` from it). When opening a PR, target that same base (e.g. `gh pr create --base {{ workflow.pr_base_branch | default: "main" }} --body "Fixes #N"`). You MUST (1) push your branch, (2) open a PR with body containing "Fixes #N", (3) post a comment on this issue with the PR URL, (4) add the `pr-open` label to this issue (e.g. `gh issue edit --add-label pr-open`). Do not consider the task complete until all four are done. Use `gh pr create` and `gh issue comment --body "PR: <url>"` if available. Do **not** merge the PR—a human merges; closing the issue happens when the PR is merged. The orchestrator uses the pr-open label to find issues with an open PR for fix-PR re-dispatch (e.g. when CI fails or someone requests changes).

---

# Symphony workflow — RustSymphony

You are working on a GitHub issue for the **RustSymphony** project (a Rust implementation of the Symphony orchestrator).

## Issue

- **Title:** {{ issue.title }}
- **Identifier:** {{ issue.identifier }}
- **State:** {{ issue.state }}
{% if issue.url %}
- **URL:** {{ issue.url }}
{% endif %}
{% if issue.labels.size > 0 %}
- **Labels:** {{ issue.labels | join: ", " }}
{% endif %}

{% if issue.description %}
## Description

{{ issue.description }}
{% endif %}

{% if attempt %}
## Attempt

This is **attempt {{ attempt }}**. A previous run may have been interrupted or failed; continue from the current state of the git worktree.
{% endif %}

## Instructions

1. **Claim the issue** (if the workflow uses a claim label): Add the claim label to this issue first (e.g. `gh issue edit … --add-label symphony-claimed`) so no other worker picks it up.
2. Read the issue and the codebase in this git worktree.
3. **Fix-PR runs (SPEC_ADDENDUM_2 B.7):** If this issue already has an open PR (e.g. you were re-dispatched because CI failed or someone requested changes): use the **current branch**, pull or rebase as needed, make changes to address the failure or review feedback, commit and push. Do **not** open a new PR. When done, you may post a short comment on the issue or PR (e.g. "Pushed fixes for CI.").
4. **New work:** Otherwise, implement what the issue asks for. Use a single branch per issue (e.g. `symphony/issue-<number>`), created from the base branch (e.g. `main` or `{{ workflow.pr_base_branch | default: "main" }}`), if opening a PR.
5. Run tests and fix any failures (`devenv shell -- moon run :test` as appropriate).
6. Follow project conventions (see `.cursor/rules`, `docs/`, and existing code).
7. When done, either:
   - **PR-driven:** You MUST complete all of: (1) push your branch, (2) open a PR with body "Fixes #N" targeting the base branch (e.g. `gh pr create --base main --body "Fixes #N"` or use `--base {{ workflow.pr_base_branch | default: "main" }}` when the variable is set), (3) post a comment on this issue with the PR link (e.g. `gh issue comment <number> --body "PR: https://github.com/…"`), (4) add the `pr-open` label to this issue (e.g. `gh issue edit --add-label pr-open`). Do not exit until all four are done. Do not merge the PR—a human merges. **Or**
   - **Direct close:** Summarize in a comment, then **close this issue** (e.g. `gh issue close` or GitHub UI) so the runner stops picking it up. If you cannot close issues, add a clear "ready to close" comment for a maintainer.

## Before you're done (required for PR-driven flow)

If you are using the PR-driven handoff, you must complete every item before exiting:

- [ ] Branch pushed to origin
- [ ] PR opened with body containing "Fixes #N" (replace N with the issue number from this ticket)
- [ ] Comment on this issue with the PR link
- [ ] Label `pr-open` added to this issue (so the orchestrator can re-dispatch on failing checks or review requests)
