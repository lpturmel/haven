---
name: factory
description: Run this project's software factory. `/factory run` drains the backlog into merged code through intake, spec, build, gate, review and approval. `/factory run <id>` drives just that one job. `/factory status` lists jobs. `/factory approve <id>` approves a job waiting on you. `/factory reject <id> <why>` rejects one. `/factory <anything else>` sends your words to a job's builder.
argument-hint: run [<id>] | status | approve <id> | reject <id> <why> | <feedback for a job>
allowed-tools: Bash(factory/bin/job:*), Bash(factory/bin/backlog list), Agent, Read
---

# Factory orchestrator

You are the one orchestrator for every job in this project's factory. You
start jobs, spawn every agent as a background subagent, and call the
scripts. **You write nothing yourself**: no Write, no Edit, no git, no
comments on issues. Every fact lives in `factory/jobs/<id>/`, and
`factory/bin/job` is the only thing that changes it. Your context stays small
because agents reply with one line and the scripts print one line; don't
read agent files unless the user asks or something is wrong.

Run every command from the project root (the main checkout). Never `cd` into a
worktree.

**Every job has its own git worktree**, which `factory/bin/job` manages: `job
next` creates it at `factory/worktrees/<id>` on branch `factory/<id>` when
intake accepts the job, and removes it once the job is merged or rejected.
Every agent after intake (spec-writer, builder, reviewers, approver) works in
that worktree, finding the path in `job.json`; intake runs before it exists.
So spawn agents **without** the Agent tool's `isolation` option: a worktree
of its own would put the agent's work outside the job's worktree and branch.

## Arguments: `$ARGUMENTS`

- `run` (or empty) → [Run](#run).
- `run <id>` → [Run one job](#run-one-job). `<id>` is a job id or a backlog
  ref (e.g. `gh-7` or `7`).
- `status` → run `factory/bin/job list --all` and show it as a short table
  (id, stage, round, note). Stop.
- `approve <id>` → [Approve](#approve).
- `reject …` and anything else → [Talk to a job](#talk-to-a-job).

Before putting any `<id>` from the user on a command line, check that it
matches `[A-Za-z0-9._-]+` (letters, digits, `.`, `_`, `-`, nothing else). If
it doesn't, show the usage (`run [<id>] | status | approve <id> | reject <id>
<why> | <feedback for a job>`) and run no command.

## Run

1. `factory/bin/job sync` — starts new jobs from the backlog up to
   `concurrency` and prints `<id> <stage>` for every open job. This is also
   how a fresh session resumes: whatever is in `factory/jobs/` continues from
   its stage. If `sync` prints `ERROR <message>`, **stop the whole run**: run
   no `next` for any job and tell the user the message. For
   `ERROR run /factory from <folder> ...`, this session is not in the main
   checkout (e.g. a worktree session) and project permissions don't apply
   here: tell the user to open a Claude Code session in that folder and run
   `/factory` again.
2. For every printed job that has **no agent of yours still running**, run
   `factory/bin/job next <id>`. It does the deterministic work for that stage
   (routing, worktree, push, draft PR, gate, merge) and prints one line.
   `next` can take minutes when it runs the gate; run it with
   `run_in_background` when other jobs are waiting on you.
3. Act on the line:
   - `<id> <stage> SPAWN <agent>:<model>[:<effort>] [<agent>:<model>[:<effort>] ...] out=<f>[,<f>...]`
     → split each token on `:` into agent, model and an optional effort.
     Spawn each listed agent **in the background**, all at once (reviewers
     run in parallel), using the Agent tool with `subagent_type` = agent,
     `model` = model, `effort` = the third field when the token has one (pass
     no `effort` at all when it has only two fields), and exactly this prompt
     (paths absolute, `<n>` from `factory/bin/job show <id> round` only if you
     don't already know it):

     ```
     Factory job: <absolute path to factory/jobs/<id>>
     Agent: <agent>
     Round: <n>
     Write: <absolute path to factory/jobs/<id>/<its out file>>

     Follow your agent instructions. Read job.json first.
     ```

     The first line is how per-job cost is attributed; never change it. The
     n-th agent gets the n-th `out` file.
   - `<id> <stage> WAIT <author|human> <note>` → nothing to do for this job now.
   - `<id> <stage> DONE <merged|rejected>` → the job is finished.
   - `ERROR <id>: <agent> did not write <file>` → the agent was spawned twice
     and its out file still doesn't exist. Stop working that job and tell the
     user. The likely cause is that the agent's Write was denied (for example
     a session running outside the main checkout, or a permission prompt that
     was refused). Don't re-spawn the agent and don't run `job retry` on your
     own; the user decides (see **Retry** below).
   - `ERROR <message>` → stop working that job and tell the user the message.
     Don't try to fix state by hand.
4. When a background agent finishes, note its one-line reply. **Wait for all
   of a job's running agents** (e.g. every reviewer) before calling
   `factory/bin/job next <id>` for that job again; then go to step 3.
   - If an agent's one-line reply starts with `ERROR` (e.g.
     `ERROR code-reviewer could not write <file>: no Write tool`), that job
     is **stopped**. Still wait for that job's other running agents to
     finish, but then do not call `factory/bin/job next <id>` for it and do
     not respawn any agent for it. Tell the user which agent failed and its
     message. The user decides, for example with `/factory run <id>` or a
     retry (see **Retry** below). Other jobs carry on as normal.
5. Whenever a job reaches `WAIT` or `DONE`, run `factory/bin/job sync` again
   so a freed slot starts the next backlog item, and `next` any new job. If
   that `sync` prints `ERROR`, handle it as in step 1: stop and tell the user.
6. Stop when no agent of yours is running, every open job is `WAIT`,
   `ERROR` or stopped by an agent's `ERROR` reply (step 4), and `sync`
   started nothing new. Report a short table: each job,
   its stage, and what it waits on (for `needs-human`, the note and the PR
   link from `factory/bin/job show <id> pr`). Mention that the user can reply
   with `/factory <feedback>`, `/factory approve <id>`,
   `/factory reject <id> <why>` or `/factory run <id>`.

Never spawn the same agent for the same job and output file twice. If you
lost track after a compaction, check your running background tasks before
re-spawning anything `next` asks for.

## Run one job

Drive one job forward without touching any other job or pulling new work. It
ignores `concurrency` for this job.

1. `factory/bin/job pick <id>` — resolves `<id>` (a job id or a backlog ref)
   to one job, creating it if `<id>` is a ready backlog item with no job yet.
   It prints `<id> <stage>`, or `ERROR <message>`: then show the message and
   stop.
2. Loop [Run](#run) steps 2–4 for the printed id **only**. Never run
   `factory/bin/job sync`, and never run `next` for any other job, not even
   after this one finishes.
3. Stop when that job's `next` prints `WAIT`, `DONE` or `ERROR`, or one of
   its agents replied with an `ERROR` line (Run step 4), and none of its
   agents is running. Report its stage and what it waits on (for
   `needs-human`, the note and the PR link from `factory/bin/job show <id> pr`),
   and the same reply commands as Run step 6.

## Approve

Approval is only ever this explicit command.

- No `<id>`: show the usage `/factory approve <id>` and the `needs-human` jobs
  from `factory/bin/job list`. Run nothing else.
- Otherwise run `factory/bin/job approve <id>` (a job id as shown by
  `/factory status`) and act on its line as in Run step 3. If it prints
  `SPAWN` (e.g. a merge conflict sent it back to the builder), continue the
  loop of [Run one job](#run-one-job) step 2–3 for that job only. If it prints
  `ERROR` (unknown job, or not `needs-human`), show it to the user; nothing
  changed.

## Talk to a job

The user's words are about one job. Find it from `factory/bin/job list --all`
by id, issue number or title. If more than one open job could match, ask
with AskUserQuestion which one. Then decide what they want:

- **Feedback** (change something, fix something, a different approach):
  pass their words verbatim on stdin:

  ```bash
  factory/bin/job feedback <id> <<'FACTORY_EOF'
  <the user's words, unedited>
  FACTORY_EOF
  ```

  If it prints `SPAWN`, spawn the builder as in Run step 3 and continue the
  run loop for that job. If it prints `OK … recorded`, the builder of the
  next round reads it.
- **Reject** ("drop", "reject", "won't do"): pass their reason the same way
  to `factory/bin/job reject <id>`. The reason goes on the issue, the PR
  closes, the job is done.
- **Retry** ("retry", "try again", typically after a `did not write` ERROR
  from `next`, or an agent that replied `ERROR <agent> could not write ...`):
  `factory/bin/job retry <id>`, then `factory/bin/job next <id>`, and act on
  its line as in Run step 3.

This path never approves. If the words read as an approval ("merge it",
"ship it", "approve", "lgtm"), run nothing and reply with the exact command
to type: `/factory approve <id>`.

If it's unclear whether they mean feedback or reject, ask. Never add your
own words to theirs.
