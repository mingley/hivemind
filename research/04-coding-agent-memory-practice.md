# Research 04: Coding-agent memory and coordination in practice

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim. Evidence labels: **[V]** verified in a primary source, **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

- **Vendors have converged on one design.** Copilot, Claude Code auto memory, Codex Memories, Devin Memory and the open Agent Memory Repo spec all work the same way. A small index is loaded every session: `MEMORY.md`, `memory_summary`, or an AGENTS.md "table of contents" of about 100–200 lines or 9–25 KB. Detailed notes are read only when needed. Storage is plain files or a database, often versioned in git, and the agent writes the memories itself. Several products also run a background clean-up pass (Codex Phase 2, Devin "Dreaming").
- **Only GitHub Copilot Memory checks memories against the current code.** Each memory carries code citations, and the agent checks them when it uses the memory.
  - Copilot upvotes or downvotes memories. Retention is 90 days by default (configurable 7–180) and resets only when Copilot upvotes a memory, not when it merely retrieves it.
  - It rejects memories containing secrets or model-control tokens, and it gates creation and use on repository write and read access.
  - GitHub's own A/B tests: coding-agent PR merge rate 90% vs 83%, and code-review positive feedback 77% vs 75%. These are vendor figures and nobody has reproduced them.
  - It also ships a JSONL import format with file and line-range citations, which could serve as a ready-made exchange format.
- **Memory is siloed by tool, machine and person.** Claude Code auto memory is machine-local and deliberately skips anything derivable from the code, including debugging fixes. Codex Memories is local. Devin Memory is personal. Copilot facts are scoped to one GitHub repository.
- **Products keep changing:**
  - Cursor removed Memories in 2.1.17.
  - Windsurf's Cascade agent, which owned its memories, was removed.
  - Gemini CLI removed its `save_memory` tool.
  - Devin replaced Knowledge with Skills.
  - Continue's repo is now read-only, and Jules shuts down at the end of 2026.
  - OpenMemory was removed from the mem0 repo, and Cipher's repo is archived.
- **Independent studies are sobering:**
  - Context files don't reliably raise task success and add more than 20% to inference cost (ETH Zurich).
  - When an instruction file contradicted the code, agents followed the stale file 90.1% of the time.
  - Agents never repaired stale instruction files.
  - In VibeMemBench, existing memory systems failed to beat the no-memory baseline in 11 of 12 pairings. Injecting already-verified experience directly did help, by 1.1–4.5 percentage points.
- **Open-source adoption is uneven:**
  - claude-mem (~116k★) leads in stars but has reports of runaway token use.
  - Beads (~28.7k★) is the de facto task ledger for agents and now stores data in Dolt.
  - MCP Agent Mail offers file reservations with time limits and agent mailboxes, but users report SQLite corruption under load, and its Rust rewrite has a license rider that excludes OpenAI and Anthropic.
  - No open-source tool has an independent evaluation of coding outcomes.
- **Coordination evidence:**
  - Anthropic's 16-agent C-compiler project claimed tasks by committing lock files through git.
  - Cursor found locks became a bottleneck: 20 agents slowed to the throughput of 2–3. Optimistic concurrency worked better, and a planner/worker/judge hierarchy worked best.
  - Claude Code agent teams use file-locked task claims and JSON mailboxes. Its cross-session messaging lets one session notify another of changes.
- **What this suggests for hivemind [Inf]:** build a provenance-first memory and coordination service that checks memories when they are read. It should plug into the surfaces agents already have rather than add one more always-loaded file:
  - the Anthropic memory-tool handler;
  - Claude Code's `autoMemoryDirectory` setting;
  - Copilot's JSONL import format;
  - the Agent Memory Repo layout.
  
  It should present memory to the model as evidence rather than instruction. The baseline it has to beat is a git repository of markdown notes plus Beads.

---

## 04 — Coding-agent memory and multi-agent coordination in practice (checked 2026-11-10)

**Labels:**
- **[V]** verified in a primary source (cited).
- **[C]** vendor or project claim.
- **[I]** independent third-party evidence.
- **[Inf]** my inference.
- **[?]** unknown or not verified.

**Method:** public sources only. Product docs, changelogs, engineering posts, public GitHub repos (read through raw/API URLs), arXiv and public issue trackers. Nothing was cloned, built or run. Bracketed numbers [n] refer to §11.

---

## 1. Summary

1. **One shape everywhere.** A small always-loaded index plus detail files read on demand. Claude Code `MEMORY.md` loads its first 200 lines or 25 KB [5]. Codex `memory_summary` is capped at 8,900 bytes [13][14]. Devin and Agent Memory Repo load `MEMORY.md` [26][29]. OpenAI uses a ~100-line AGENTS.md as a table of contents [100] [V].
2. **Copilot Memory is the only shipped system that checks memories against code when they are used.** Memories carry citations, votes and a 90-day retention period that resets on upvote (configurable 7–180 days). It rejects secrets and model-control tokens, and gates on repository permissions [1][2] [V].
3. **Copilot's reported gains are vendor-only.** PR merge rate 90% vs 83%; review feedback 77% vs 75%; offline review precision +3% and recall +4% [1] [C]. No independent replication [?].
4. **Copilot CLI imports memories from versioned JSONL** with `{scope, subject, fact, reason, citations[file,lineStart,lineEnd | text]}` [3] [V]. This is an existing exchange schema hivemind could accept.
5. **Claude Code runs two systems:**
   - Human-written CLAUDE.md/AGENTS.md, delivered "as a user message after the system prompt".
   - Machine-local auto memory with four note types (user, feedback, project, reference).
   - Auto memory deliberately skips facts derivable from code, "such as architecture, file paths, or debugging fixes" [5] [V].
6. **Anthropic's API memory tool runs on the client.** It is a file directory: `view`, `create`, `str_replace`, `insert`, `delete`, `rename` under `/memories`. The developer implements storage, expiry and path-traversal defences [9] [V]. Anthropic's internal eval: memory tool plus context editing +39%; context editing alone +29% and −84% tokens on a 100-turn task [10] [C].
7. **OpenAI Codex Memories** is a local, two-phase LLM pipeline:
   - Phase 1 extracts memories from each past session.
   - Phase 2 consolidates them in a git-versioned `~/.codex/memories`, with a single global lock.
   - It ranks by usage and recency, drops memories unused beyond a window, redacts secrets, and tells the model "memory is not proof of current behavior" [12][13] [V].
8. **Product churn is high.** Cursor removed Memories in 2.1.17 [20]. Windsurf's Cascade agent (and its memories) was removed [24]. Gemini CLI removed `save_memory` [46]. Devin moved Knowledge to Skills and launched personal git-backed Memory with nightly "Dreaming" [26][28]. Users lose memories, or must export them, when products change [Inf].
9. **Independent evidence on always-loaded context files is negative to mixed:**
   - No general gain and more than 20% added cost [107].
   - Median runtime −28.6% and output tokens −16.6% at comparable completion [106].
   - No measurable correctness change [108].
   - Stale files override code in 90.1% of choices [109]; agents never repair them [110] [I].
10. **Memory systems mostly don't help coding yet.** In VibeMemBench, Mem0, SimpleMem, MemoryOS and A-MEM failed to beat the memory-off baseline in 11 of 12 pairings. Directly injecting experience already verified by execution raised resolution by 1.1–4.5 percentage points [112] [I].
11. **Open-source coding-agent memory falls into two families:**
    - **Work ledgers:** Beads, Task Master, Backlog.md, Agent Mail, Gas Town.
    - **LLM-extracted knowledge stores:** claude-mem, mem0, Letta, Graphiti, ByteRover, cass.
    
    I found no independent outcome evaluations for any of them, and none of them check memories against code [52]–[92] [V]/[?].
12. **Poisoning is demonstrated, not hypothetical.** Windsurf's auto-memory persisted prompt injection [114]. Rules files can be backdoored [116]. Optimized injections in rule files swap in attacker packages [113]. A Context7 docs response carried text injected for the model [82] [I].
13. **Coordination primitives exist:**
    - Atomic claims, typed dependencies and a "ready" queue (Beads [55][56]).
    - Time-limited advisory file reservations and mailboxes (Agent Mail [67][68]).
    - File-locked task claims and JSON inboxes (Claude agent teams [7]).
    - Cross-session messages (Claude Code [8]).
    
    The best evidence is practitioner write-ups. Git lock files worked for 16 agents [99]. Locks failed at scale and hierarchy won [104] [C/I].
14. **Proven context-engineering moves:** compaction; structured note-taking; just-in-time retrieval by reference; subagents returning 1–2k-token summaries [93]; progressive disclosure of skills [95][96]; on-demand tool loading (150k → 2k tokens [97]); restorable compression and a stable prefix for the prompt cache [103]. These are vendor-documented [V], with vendor metrics [C].
15. **Bottom line for hivemind [Inf]:** nothing on the market combines cross-tool, cross-machine and cross-agent shared memory, provenance and checking at read time, coordination primitives, and a permissive license. The closest are Copilot Memory (closed, GitHub-only) and git-markdown memory repos (no verification). A git repo plus Beads is the baseline hivemind's evaluations must beat.

---

## 2. Product memory features (claimed / shipped / demonstrated)

### 2.1 GitHub Copilot Memory (repo facts + user preferences)
**Shipped [V]:**
- **History.** Announced in public preview for the coding agent, CLI and code review on Jan 15, 2026, off by default [1].
- **Current scope.** Docs (checked 2026-11-10) now describe two memory types [2]:
  - **Repository-level facts:** shared with collaborators, created only by users with write access, used only on that repository, with code citations.
  - **User-level preferences:** follow the user across repositories. On Business/Enterprise plans they are owned by the billing entity, and admins can export or delete them.
- **Surfaces.** The Copilot app, CLI, code review, cloud agent (the docs' current name for the coding agent), Agentic Autofix, Mobile, Slack and Teams.
- **Defaults.** On for Pro/Pro+/Max; off for Business/Enterprise until enabled by policy [2].
- **Creation.** A `store_memory` tool call made by the agent [1]. Users can prompt Copilot to remember, but cannot create or edit memories directly in settings. Users can bulk-import JSONL through `copilot memories import` with `--dry-run` [3].
- **Write permissions.** In the CLI, only the root agent may store or vote; subagents can only read [3]. Interactive CLI sessions can ask for approval before a memory is stored [3].
- **Unit.** `{subject, fact, citations, reason}`, plus agent, model, billing entity, scope, voting data and expiry time [1][2].
- **Validation.** Before applying a memory, the agent is "prompted to verify its accuracy and relevance by checking the cited code locations". If the code contradicts the memory, the agent stores a corrected version; if it confirms the memory, it re-stores it to refresh the timestamp [1].
  - The docs add explicit votes: upvoted memories are more likely to be sent to the model, and memories downvoted past a threshold stop being used and are deleted after a period of disuse [2].
  - The docs warn this is "a judgment Copilot makes, rather than a guarantee enforced by the system" [2].
- **Expiry.** 90 days by default. The clock resets on upvote, not on retrieval. Admins can set 7–180 days for repo facts, and the lower of the org and enterprise values applies [2].
- **Write-time filters.** Memories containing detected secrets are rejected; "recognized model-control tokens" can be rejected as a prompt-injection defence; the agent is told not to store sensitive or short-lived information [2].
- **Branches.** Facts can be captured from PRs that were closed without merging, and verification may not catch every outdated fact [2].
- **Retrieval.** At launch: "the most recent memories for the target repository" were put into the prompt, with a search tool and weighting planned [1]. Now: not all memories are sent, and upvoted ones are favoured [2].
- **Relation to instructions.** Memories sit alongside custom instructions: `.github/copilot-instructions.md`, path-specific `.github/instructions/**/*.instructions.md`, and AGENTS.md / CLAUDE.md / GEMINI.md agent instructions [4]. The docs say to use custom instructions for anything that must apply every time [2][3].

**Demonstrated [C]:**
- Seeded adversarial memories "self-healed" as agents checked citations [1].
- Offline code-review eval: precision +3%, recall +4%.
- A/B tests: coding agent merge rate 90% vs 83% (reported as a "7% increase"; it is 7 percentage points); review positive feedback 77% vs 75%; p < 0.00001 [1].
- All vendor-run. No independent evaluation found [?].

### 2.2 Claude Code (CLAUDE.md hierarchy, rules, auto memory, subagent memory)
**Shipped [V] [5]:**
- **CLAUDE.md locations.** Managed policy file; `~/.claude/CLAUDE.md`; `./CLAUDE.md` or `./.claude/CLAUDE.md`; `CLAUDE.local.md`.
- **Loading.**
  - Files in ancestor directories load at launch, concatenated root-first.
  - Files in subdirectories load the first time Claude reads a file there.
  - `@imports` nest up to 4 hops and "don't reduce its context cost".
  - HTML block comments are stripped.
  - Files over 4 MiB are skipped.
- **Size guidance.** "Target under 200 lines per CLAUDE.md file. Longer files consume more context and reduce adherence."
- **Delivery.** Content arrives "as a user message after the system prompt" and is "context, not enforced configuration". Hooks are the recommended route for anything that must be enforced.
- **Rules.** `.claude/rules/*.md` files with a `paths:` glob load only when a matching file is touched.
- **AGENTS.md.** Read natively from v2.1.277, by default only when no CLAUDE.md is present.
- **Staleness audit.** `/doctor prompt-audit` looks for "references to files or commands that don't exist" and contradictions, and proposes edits.
- **Auto memory.**
  - On by default for local sessions.
  - Stored at `~/.claude/projects/<project>/memory/`: a `MEMORY.md` index (one line per memory) plus topic files with `type` frontmatter (user/feedback/project/reference).
  - The first 200 lines or 25 KB of `MEMORY.md` load each session; topic files are read on demand.
  - Claude Code nags the agent to shorten the index when it nears the limit, and records a `modified` timestamp on each write.
  - Explicitly **machine-local**, shared across the worktrees of one repo.
  - Excluded from the transcript clean-up sweep.
  - `autoMemoryDirectory` can relocate it to any absolute path.
  - It skips "anything it can derive from the codebase, such as architecture, file paths, or debugging fixes".
- **Subagent memory.** Subagents get their own `memory:` directory with scope `user`, `project` (committed to git, the recommended default) or `local`. The first 200 lines or 25 KB of its `MEMORY.md` are injected [6].

**Demonstrated:** no published outcome metrics for CLAUDE.md or auto memory [?]. See §4.1 for Anthropic's general context-management numbers.

### 2.3 Anthropic API: memory tool, context editing, compaction
**Shipped [V] [9]:**
- **Interface.** `{"type":"memory_20250818","name":"memory"}`, a stable tool. Storage is client-side: the app implements `view`, `create`, `str_replace`, `insert`, `delete` and `rename` under `/memories`, and maps that prefix to any backend. SDK helpers exist, including `BetaLocalFilesystemMemoryTool`.
- **Auto-injected protocol.** The API adds this to the system prompt: "ALWAYS VIEW YOUR MEMORY DIRECTORY BEFORE DOING ANYTHING ELSE… ASSUME INTERRUPTION."
- **Security is the developer's job:** strip sensitive data, cap file sizes and `view` output, "periodically delete memory files that haven't been accessed in a long time", and defend against path traversal.
- **Documented pattern.** A "multisession software development pattern": an initializer session writes a progress log, a feature checklist and an init script; every later session reads them and updates the log at the end.
- **Pairings.** Combine with context editing (clears stale tool results on the client) and server-side compaction [9][10].

**Claimed [C] [10]:** on an internal agentic-search eval, memory plus context editing +39% and context editing alone +29%; on a 100-turn web-search eval, context editing let runs finish while cutting tokens 84%.

**Integration note [Inf]:** because storage is client-side, hivemind could be the backing store behind this tool with no model-side changes.

### 2.4 OpenAI Codex: AGENTS.md and Memories
**AGENTS.md [V] [11]:**
- **Discovery.** Global `~/.codex/AGENTS.md` (or `AGENTS.override.md`), then one file per directory from the repo root down to the working directory, concatenated root-first.
- **Size cap.** Concatenation stops at `project_doc_max_bytes`, 32 KiB by default.
- **Governance.** The AGENTS.md standard is stewarded by the Agentic AI Foundation under the Linux Foundation, announced 2025-12-09 [15][16] [V]. The claim of "over 60k open-source projects" is [C] [15].

**Memories [V] [12][13][14]:**
- **When it runs.** Root, non-ephemeral, non-subagent sessions trigger a background pipeline.
- **Phase 1.** Claims idle past sessions via leases in the state database. A model extracts `raw_memory`, `rollout_summary` and `rollout_slug`, and secrets are redacted.
- **Phase 2.**
  - Takes a single global lock and selects top-N memories by `usage_count` and recency, ignoring those unused beyond `max_unused_days`.
  - Syncs `raw_memories.md` and `rollout_summaries/` into a git-baselined `~/.codex/memories`.
  - Runs a consolidation subagent with "no approvals, no network, and local write access only", which maintains `MEMORY.md`, `memory_summary.md` and `skills/` [13].
- **Context cost.** Memory context is truncated to 8,900 bytes, "capped below 10k tokens" [14].
- **Read path.** Answers that use a memory must cite it. The prompt says "Memory is not proof of current behavior", and memory is updated only when the user asks (as reported in the sub-pass, [12]).
- **Defaults and sharing.** Off by default. Local per Codex home. The docs keep team guidance in AGENTS.md and treat memories as a personal "recall layer" [12].
- **History.** First commit 2026-02-10. "Memory v2 by default for fresh profiles" (changelog, 2026-10-27), as reported in the sub-pass [12].

**Demonstrated:** no published metrics [?].

### 2.5 Cursor
**Shipped [V] [17]:**
- **Rule types.** Project rules `.cursor/rules/*.mdc`, applied Always, by file glob, "Apply Intelligently" (the agent decides from the description), or Manually. Plus User Rules, dashboard Team Rules (which can be "Enforced"), and nested AGENTS.md.
- **Guidance.** "Keep rules under 500 lines."
- **Memories history:**
  - Beta in 1.0 (Jun 4, 2025): "remember facts from conversations… per project on an individual level" [18].
  - GA in 1.2 with "user approvals for background-generated memories" [19].
  - **Removed in 2.1.17.** A Cursor forum reply (2025-11-24) points users to "Export memories" into an `.mdc` rules file [20] [V, forum]. The replier's staff status is unverified, and a later reply contradicts it (2026-01-10: only the UI was removed) [21] [?].
  - No changelog entry found [?].
- **Newer.** The Agent SDK `memoryHook` writes a JSONL journal that the docs warn is shared and untrusted [22] [V, sub-pass].

**Demonstrated:** none [?].

### 2.6 Windsurf / Cascade → Devin Desktop
**Was shipped [V]:**
- Cascade auto-generated, workspace-scoped memories "if it encounters context that it believes is useful"; `.windsurf/rules` capped at 12,000 characters [23].
- Rehberger showed that `create_memory` ran without approval and could persist injected instructions [114] [I].

**Now [V] [24][25]:**
- 2026-07-29: a Cascade Migration Wizard migrates memories.
- Changelog: "Cascade has been removed. Devin Local is now the only agent."
- FAQ: Devin Local "replaces memories and workflows with rules and skills".

### 2.7 Devin (Cognition): Memory and Dreaming; Agent Memory Repo spec
**Shipped [V] [26][28]:**
- **Unit.** A personal "memory drive": a git repo of markdown with `MEMORY.md` as entry point and one-line entries tagged `[source: <session-id>; added: <date>]`.
- **Loading.** Each session gets a checkout, receives `MEMORY.md` as context, and searches or reads other notes on demand.
- **Concurrency.** "Parallel sessions can write to the same drive: a stale write is rejected and retried, and conflicting edits are resolved rather than overwritten."
- **Dreaming.** Runs about daily. It merges overlapping notes and "removes transient details and stale notes that no session has used".
- **Sharing.** Memory is personal and not shared with the org; Skills are the shared layer.
- **Knowledge.** Trigger-described items, auto-suggested and scoped to org or repo [27]. Now migrating to Skills (2026-09-18) [28].
- **Open spec.** Built on **Agent Memory Repo** (MIT; 859★; created 2026-10-04) [29]. The memory loop is clone → grep → update → push after every edit. Listed use cases include agent swarms ("git flags any edits that conflict") and team memory [29] [V].

**Demonstrated:** none [?].

### 2.8 Augment Code
**Shipped [V]:**
- User and workspace rules, plus hierarchical AGENTS.md/CLAUDE.md. Limits: 24,576 characters for user guidelines; 49,512 for workspace guidelines plus rules [30].
- "Agent Memories" (2025) with a "Memory Review" approval flow [31] [C]. Their current IDE status is unclear [?].
- Cosmos "Experts" memory separates "simple" trusted feedback from "noisy" memory kept as an evidence log and promoted once evidence is strong [33] [V, sub-pass].
- Context Engine MCP: semantic code retrieval [32].

**Claimed [C]:**
- Context Engine MCP gives +30–80% across agents on a self-built 900-attempt benchmark [32].
- SWE-bench Pro 51.8% [118].
- Not independent [?].

### 2.9 Amp
**Shipped [V]:**
- **AGENTS.md handling.** Loaded from the working directory up to `$HOME`, plus user, system and admin "Global AGENTS.md" files. Subtree files load when a file in the subtree is read. @-mentioned files can be scoped by globs [34].
- **Handoff.** Introduced 2025-10-23 as "No More Compaction": it generates a goal-directed prompt and a list of relevant files for a fresh thread [35].
- **Compaction is back.** The CLI was rebuilt as "compaction-first" in May 2026 [36].
- **Thread sharing.** Public discoverable threads ended 2026-06-02 over leak risk [37].
- No persistent memory store [?].

### 2.10 Cline
**Shipped [V] [38][39]:**
- Memory Bank is a *methodology*, not a built-in feature: `memory-bank/{projectbrief,productContext,activeContext,systemPatterns,techContext,progress}.md`, driven by custom instructions in `.clinerules`.
- `/newtask` hands work off to a fresh task. `/smol` and Auto Compact summarize near the context limit, and compaction can be undone through checkpoints.

**Demonstrated:** none [?].

### 2.11 Aider
**Shipped [V] [40][41]:**
- The repo map uses tree-sitter definition and reference tags and personalized PageRank (identifiers mentioned in chat ×10; references from files already in chat ×50).
- It binary-searches how much to include to fit `--map-tokens`, which defaults to 1k.
- Conventions are loaded with `--read CONVENTIONS.md`.
- This is retrieval over code, not memory. It is a useful model of budgeted, ranked context [Inf].

### 2.12 Continue
**Shipped [V] [42][43]:**
- `.continue/rules` with `alwaysApply`, globs, regex and description; the agent can write rules (`create_rule_block`).
- **Status.** The site says "acquired by Cursor"; the repo is read-only after a final 2.0.0 release.

### 2.13 Gemini CLI and Jules
**Shipped [V] [44]–[47]:**
- **GEMINI.md.** A hierarchy plus just-in-time files, all "sent… with every prompt", with `@imports`.
- **`save_memory` removed.** It used to append to `## Gemini Added Memories`; PR #26941 (2026-05-13) removed it. The agent now edits memory files directly.
- **Experimental Auto Memory.** Scans idle sessions (≥3 h, ≥10 user messages) and drafts `.patch` and `SKILL.md` files into an inbox, which the user reviews with `/memory inbox`.
- **Jules.** Added per-repo memory in 2025-09. Winds down 2026-12-31 [47].

### 2.14 Briefly [V, sub-pass]
- **Kiro:** steering files with always / fileMatch / manual / auto inclusion [48].
- **Factory:** AGENTS.md caps of 80,000 characters at start and 40,000 per dynamic load [49].
- **JetBrains Junie:** `.junie/AGENTS.md` plus rules; imports other agents' files [50].
- **Zed:** "Rules have been replaced by Skills and Instructions"; reads the first matching rules/agent file [51].

**Takeaways [Inf]:**
- Every product separates *human-curated rules/instructions* (always-on, shared via git) from *agent-curated memory* (usually personal, local or cloud).
- "Skills" are becoming the vehicle for procedural memory: Devin Knowledge→Skills, Codex consolidation writing `skills/`, Gemini Auto Memory drafting `SKILL.md`.

---

## 3. Open-source memory and coordination tools for coding agents

Stars and versions were checked 2026-11-10 through the GitHub API (I verified key repos myself).

| Tool | Unit / storage | Creation | Validation / expiry | Retrieval / token cost | Coordination | Evidence / issues |
|---|---|---|---|---|---|---|
| **Beads** (`steveyegge/beads` → `gastownhall/beads`; 28,664★; MIT) [52] | Issue "beads" with hash IDs (`bd-a1b2`), now in a **Dolt** database; `issues.jsonl` is "an export… not the source of truth"; `bd remember` key/value memories [52] [V] | Agent through CLI; `bd setup <agent>` installs hooks and AGENTS.md text | `bd compact` Tier 1 LLM-summarizes issues closed 30+ days; Tier 2 planned [53]; no staleness checks on memories; the open "Memory Beads" proposal says "loading every remembered body spends context" [57] | `bd ready` lists unblocked work; `bd prime` injected by hooks; memories appended uncapped unless capped, because "hosts truncate silently" [54]; ~50 tokens (MCP) or 1–2k (CLI) [C] | Typed blocking dependencies; `--claim` atomic claims; per-project "merge slot" [55][56] | No outcome metrics. Dolt-migration data loss and sync issues; JSONL-era merge conflicts [58] |
| **Gas Town** (18,760★) [59] | Orchestrator on top of Beads: Mayor, Polecats (workers), mail, handoffs, git-worktree hooks | — | — | — | Mailboxes, convoys, watchdogs | "Scales comfortably to 20-30 agents" [C] |
| **claude-mem** (115,886★; Apache-2.0) [60] | SQLite `observations` (title, facts, narrative, files_read/modified, content_hash, relevance/reinforcement fields) + FTS5 + Chroma [61][62] | **LLM-extracted automatically** from hook events by an "observer" model (default Haiku); `<private>` tags stripped | Reinforcement columns exist (use unverified); no TTL; no code checks [?] | SessionStart injects an index of 50 observations "~800 tokens" [C]; then `search → timeline → get_observations` [63] | None | Vendor "10x token savings" [C][65]; issues report 649M and 76M+ tokens burned [64] |
| **MCP Agent Mail** (Python 2,235★, superseded; Rust 244★, "MIT with OpenAI/Anthropic Rider") [66][67][69] | SQLite (live state) + per-project git archive of markdown messages and reservations | Agents through MCP tools; human "overseer" messages | Leases expire after a TTL, `renew` extends them [68]; identities ephemeral | Inbox and thread resources; 45 tools and 25 resources (schema cost unquantified) | **Advisory exclusive/shared glob reservations with TTL**, conflict check, pre-commit guard ("Advisory, not mandatory"), contacts, build slots [67][68] | Author stress tests only [C]; SQLite corruption at ~10+ writers, lost writes, busy loop [70] |
| **cass_memory_system** (461★; same rider) [71] | Episodic session-search index + diary + procedural `playbook.yaml` | SessionEnd `cm reflect` (LLM) | 90-day confidence half-life; "harmful" counts 4×; rules inverted to anti-patterns | `cm context "<task>"` | — | None |
| **Serena** (31,274★; GPL-3.0/MIT) [72] | Markdown `.serena/memories/` (project + global) with `mem:` cross-refs | Agent tools `write/edit/delete_memory`; `onboarding` seeds memories | Reference-integrity check only; guidance against "volatile line-level details" [73] | Name list injected; `mem:core` root, then follow links | None | One user's logs (n=1) [74]; Claude Code truncates Serena's instructions [74] |
| **Basic Memory** (4,549★; AGPL-3.0) [75] | Markdown with observations and `[[relations]]`, indexed in SQLite/Postgres; hybrid FTS + vector | Agent through MCP; humans in Obsidian | Checksum/mtime sync; no decay; temporal handling planned [76] | Tools only (nothing injected) | None | Own issue: "80%+ token savings" claim "has never been measured" [76] |
| **ByteRover / Cipher** (repo archived; Elastic-2.0) [77] | "Context Tree" of markdown; vendor paper [117] | LLM curation with approve/reject | Importance and recency decay [C] | "5-tier progressive retrieval" [C] | Team spaces (SaaS) | Vendor LoCoMo/LongMemEval scores [C] |
| **Pieces LTM** (closed) [78][79] | OS-level capture (screens, copy, audio), local | Automatic capture | [?] | MCP `ask_pieces_ltm` | — | None; privacy and injection surface [Inf] |
| **Context7** (64,540★; MIT client) [80] | Hosted docs index (backend private) | Crawled | Version-specific IDs | `resolve-library-id → query-docs`; "Do not call… more than 3 times per question" [81] | — | Injected promo text in a response (maintainer: an "official test", disabled) [82] |
| **MCP reference memory server** [83] | Single `memory.jsonl` graph of entities, relations, observations; lock file | Agent | None | `read_graph` returns everything | — | Issue on persistence defaults, atomic writes, quotas and redaction [84] |
| **mem0** (68,776★) [85][86] | OpenMemory removed 2026-07-29; Claude Code plugin needs hosted API | LLM extraction every 5 exchanges | — | Up to 5 memories injected on first prompt | — | Conversational benchmarks [C] |
| **Task Master** (28,344★; MIT + Commons Clause; quiet ~7 months) [87] | `.taskmaster/tasks/tasks.json` | Agent/PRD parse | — | Tool tiers of 7, 15 or 36 tools ≈ 5k, 10k or 21k tokens [C] | Dependencies, "next task" | — |
| **Backlog.md** (7,350★; MIT) [88] | One markdown file per task in `backlog/` | Agent/human, CLI + MCP | — | — | Dependencies, milestones | — |
| **Letta Code** (3,814★) [89] | Memory blocks + git-tracked "MemFS" | `/reflect`, `/sleeptime` LLM rewriting | — | — | Cloud for multi-machine | Quota-burn issue |
| **Graphiti MCP** (32,724★) [90] | Temporal knowledge graph (Neo4j/FalkorDB); bi-temporal edges invalidated rather than deleted | LLM extraction | Temporal invalidation | Search tools | `group_id` scoping | Vendor paper |
| **ruflo** (ex-claude-flow, 81,443★) [91] | AgentDB with HNSW vectors | — | — | — | Swarms, "work claims" | Self-measured [C] |
| **Vibe Kanban** [92] | Sunsetting; company shut down 2026-04-10 | — | — | — | Per-agent worktrees | — |

**Cross-cutting [V/Inf]:**
- I found no independent controlled evaluation of coding outcomes for any of these tools.
- Index-first progressive disclosure is everywhere.
- None checks memories against current code or commits.
- The common failures are backend migrations and concurrent writes (Beads, Agent Mail), hidden LLM-extraction cost (claude-mem, Letta), and host truncation (Serena, Beads).
- License and commercial drift is common: riders, Elastic, Commons Clause, archives, sunsets.

---

## 4. Lab guidance and documented patterns

### 4.1 Anthropic
- **Effective context engineering (2025-09-29) [93] [V].**
  - Context is a finite "attention budget" subject to "context rot".
  - **Just-in-time retrieval:** "maintain lightweight identifiers (file paths, stored queries, web links…) and… dynamically load data."
  - **Progressive disclosure.**
  - **Hybrid:** "CLAUDE.md files are naively dropped into context up front, while… glob and grep… retrieve files just-in-time, effectively bypassing the issues of stale indexing."
  - **Compaction:** tune for recall first, then precision; tool-result clearing is the "lightest touch".
  - **Structured note-taking** (NOTES.md, to-do lists; the Pokémon example).
  - **Subagents** return "a condensed, distilled summary… (often 1,000-2,000 tokens)".
- **Multi-agent research system (2025-06-13) [94].**
  - Multi-agent beat single-agent by 90.2% on an internal eval [C].
  - Token usage explains 80% of BrowseComp variance [C].
  - Multi-agent runs use ~15× chat tokens [C].
  - The lead saves its plan to Memory because context over 200k tokens gets truncated.
  - Subagents write to a filesystem and pass "lightweight references", avoiding a "game of telephone".
  - "Most coding tasks involve fewer truly parallelizable tasks than research" [V].
- **Agent Skills (2025-10-16) [95]; open standard [96] [V].**
  - Three levels: metadata at startup, body when relevant, files on demand. Bundled context becomes "effectively unbounded".
  - Install only from trusted sources.
- **Code execution with MCP (2025-11-04) [97].**
  - Present tools as files the agent loads on demand: "150,000 tokens to 2,000 tokens… 98.7%" [C].
  - Agents persist state and reusable functions/skills to disk; sensitive data can be tokenized so it never reaches the model [V].
- **Effective harnesses for long-running agents (2025-11-26) [98] [V].**
  - An initializer writes `init.sh`, `claude-progress.txt`, a JSON feature list with `"passes": false`, and a first git commit.
  - JSON is used because "the model is less likely to inappropriately change or overwrite JSON files compared to Markdown".
  - Work one feature at a time, leave a clean state, commit, and read the git log and progress file at session start.
- **C compiler with parallel Claudes (2026-02-05) [99] [V/C].**
  - 16 agents, ~2,000 sessions, ~$20k, 2B input tokens, a 100k-line compiler.
  - Task "locks" are text files in `current_tasks/`, synchronized through git; collisions are resolved by git.
  - Agents maintained "extensive READMEs and progress files".
  - Failure: on one monolithic task (compiling the kernel), "every agent would hit the same bug, fix that bug, and then overwrite each other's changes". Fixed by using GCC as an oracle to split the work.
  - Specialist roles: dedupe, performance, docs.

### 4.2 OpenAI
- **Harness engineering (2026-02-11) [100] [V/C].**
  - "Give Codex a map, not a 1,000-page instruction manual."
  - One big AGENTS.md "rots instantly… graveyard of stale rules… hard to verify". Replaced by a ~100-line AGENTS.md table of contents plus a `docs/` "system of record" (design docs with verification status, exec plans with progress and decision logs, a tech-debt tracker).
  - Linters and CI check freshness and cross-links; a recurring "doc-gardening" agent opens fix-up PRs.
  - "What Codex can't see doesn't exist."
  - Team claims: ~1,500 PRs and 3.5 PRs per engineer per day [C].
- **Codex Memories design.** See §2.4: usage-ranked consolidation; memory framed as not proof.

### 4.3 Google (ADK)
- **Context as a compiled view (2025-12-04) [101] [V].**
  - Separate durable storage (sessions, memory, artifacts) from the per-call working context.
  - Keep a stable prefix for caching.
  - **Artifact "handle pattern":** large payloads are referenced and expanded only for the turn that needs them.
  - **Memory recall:** reactive (`load_memory_tool`) or proactive (`preload_memory_tool`).
  - **Handoff scoping:** `include_contents` can be none or full, with "narrative casting" of prior assistant turns.

### 4.4 Others
- **Cognition, "Don't build multi-agents" (2025-06-12) [102] [V].**
  - "Share context, and share full agent traces"; "actions carry implicit decisions"; prefers single-threaded agents with a compression model.
- **Manus (2025-07-18) [103] [V].**
  - Design around KV-cache hits (cached input 10× cheaper on Sonnet; keep the prefix stable).
  - "File system as the ultimate context."
  - Compression must be *restorable*: keep the URL or path.
  - Recite a `todo.md`.
  - **Implication [Inf]:** memory injected into the prompt prefix that changes every session hurts cache hits.
- **Cursor, scaling agents (2026-01-14) [104] [V/C].**
  - Flat agents coordinating through a shared file and locks failed: locks were held too long or forgotten, and "twenty agents would slow down to the effective throughput of two or three".
  - Optimistic concurrency control was "simpler and more robust", but flat peers became risk-averse.
  - Planners, workers and a judge "solved most of our coordination problems".
  - An "integrator" role was removed as a bottleneck.
  - "Periodic fresh starts to combat drift."

---

## 5. Multi-agent coordination: what exists and what the evidence shows

| Primitive | Implementations | Evidence |
|---|---|---|
| **Atomic claims / locks** | Beads `--claim` and merge slot [55]; Claude agent teams claim tasks under file locks [7]; git lock files in `current_tasks/` [99]; Codex Phase 1/2 job leases in its state database [13] | Worked for 16 agents with git as arbiter [99] [C]. Locks were a bottleneck and brittle at ~20 agents [104] [C] |
| **Leases with time limits / file reservations** | Agent Mail exclusive/shared glob leases, `renew`, pre-commit guard [67][68] | Advisory only. SQLite corruption reported under load [70]. No outcome evaluation |
| **Optimistic concurrency** | Cursor's second design [104]; Devin memory drive rejects stale writes and retries [26] | Cursor: more robust than locks [C] |
| **Task boards / dependency graphs / ready queues** | Beads typed dependencies and `bd ready` [56]; Claude task list (pending / in-progress / completed, blocked-by) [7]; Task Master; Backlog.md | Claude docs: "Task status can lag… blocks dependent tasks" [7] [V]. No metrics |
| **Mailboxes / messages** | Agent Mail; Claude team inboxes (`~/.claude/teams/{team}/inboxes/{agent}.json`) [7]; Gas Town mail [59] | — |
| **Change notifications** | Claude Code cross-session messaging (`ListAgents`/`SendMessage`), "when a change in one session breaks what another is building on" [8] | Shipped [V]; no metrics |
| **Handoff documents** | Amp Handoff [35]; Cline `/newtask` [38]; progress file + feature list + git [98]; exec plans [100]; Claude Research saves its plan to memory [94] | Practitioner reports only |
| **Blackboard / shared memory** | Copilot cross-agent memory (code review → coding agent → CLI) [1]; Agent Memory Repo "agent swarms… git flags any edits that conflict" [29]; subagent artifacts written to the filesystem [94] | Copilot A/B results [C]; others none |
| **Hierarchy vs peers** | Planner/worker/judge [104]; lead + teammates [7]; Gas Town Mayor [59] | Cursor and Cognition both caution against flat parallel writers [102][104] |
| **Write-permission tiers** | Copilot CLI: only the root agent stores or votes; subagents read [3]; Codex skips subagent sessions for memory generation [13] | Design choice [V] |

The Claude docs say plainly: "Two teammates editing the same file leads to overwrites"; teams are for independent work [7] [V].

---

## 6. Comparison table

| System | Unit / format | Scope | Creation | Validation / expiry | Retrieval mode | Sharing | Evidence |
|---|---|---|---|---|---|---|---|
| Copilot Memory [1][2][3] | Fact + subject + reason + **code citations**; votes; expiry | Repo facts; user preferences (billing-entity-owned) | Agent `store_memory` tool; prompted; JSONL import | **Citation check at use**; up/down votes; 90-day TTL reset on upvote (7–180 configurable); secret and control-token rejection | Subset injected at start, upvoted favoured | Across Copilot surfaces and repo collaborators (GitHub only) | Vendor A/B [C] |
| Claude Code CLAUDE.md/rules [5] | Markdown; `@imports`; `paths:` rules | Managed / user / project / local | Human (`/init` drafts) | `/doctor prompt-audit` (manual) | Always loaded; path rules lazy | Git | None |
| Claude Code auto memory [5][6] | `MEMORY.md` index + typed topic files with `modified` time | Per repo, **machine-local**; subagent user/project/local | Agent (auto) | Index size nags; no TTL | Index (200 lines / 25 KB) always; topics on demand | Not across machines | None |
| Anthropic memory tool [9][10] | Files under `/memories` (client-run) | App-defined | Agent | App's job (docs suggest expiry) | Agent `view`s on start | App-defined | Internal +39% [C] |
| Codex AGENTS.md [11] | Markdown | Global / per directory | Human | None | Always (≤32 KiB) | Git | Independent: mixed [106]–[109] |
| Codex Memories [12][13][14] | DB records → `MEMORY.md`, `memory_summary`, summaries, `skills/` (git-baselined) | Per user / machine | **LLM-extracted** + consolidation agent | Unused-days window; usage rank; pruning; secret redaction | Summary injected (≤8.9 KB); tools for detail; citations | None | None |
| Cursor rules (+ Memories, removed) [17]–[21] | `.mdc` rules; team rules | User / project / team | Human; memories were background-generated with approval | Team "enforced" rules | Always / glob / description / manual | Git + dashboard | None |
| Windsurf memories (removed) [23][24] | Auto entries | Workspace, local | Agent (auto, no approval) | Manual | "When relevant" | None | Poisoning shown [114] |
| Devin Memory [26][29] | Git repo markdown, `[source; added]` | Personal per org | Agent (auto) + Dreaming | Dreaming removes unused / stale | `MEMORY.md` + search | Personal; spec supports swarms and teams | None |
| Augment [30]–[33] | Rules; memories; Cosmos evidence logs | User / workspace / org | Human; agent with review | Evidence-threshold promotion (Cosmos) | Always / auto; Context Engine retrieval | Org (Cosmos) | Vendor benchmarks [C] |
| Amp [34]–[37] | AGENTS.md; threads | User / workspace / system | Human | — | Always + glob-lazy; `read_thread`; handoff | Workspace threads | None |
| Cline Memory Bank [38] | 6 markdown files | Repo | Human + agent on command | Manual | Read on instruction | Git | None |
| Aider [40][41] | Repo map (computed) | Session | Computed | Recomputed | PageRank-budgeted map | — | None |
| Gemini CLI [44]–[46] | GEMINI.md tiers; private `MEMORY.md` | Global / project / private | Agent edits files; Auto Memory inbox | **Human review** of patches | Always + just-in-time | Git (project tier) | None |
| Beads [52]–[58] | Dolt issues + key/value memories | Repo (+ cross-repo deps) | Agent CLI | Closed-issue LLM compaction | `prime` + `ready` + search | Dolt push/pull via git remote | None; data-loss issues |
| claude-mem [60]–[65] | SQLite observations + vectors | User, partitioned by project | **LLM from hooks** | Dedupe / reinforcement fields | Index (~800 tokens [C]) + 3-layer tools | Paid cloud sync | Vendor only; token burn |
| Agent Mail [66]–[70] | SQLite + git markdown messages / leases | Project (HTTP server) | Agent | Lease TTL | Inbox tools | One server, many agents | Author stress tests |
| Serena [72]–[74] | Markdown memories | Project + global | Agent | Reference check | Name list → read | Git | n=1 logs |
| Basic Memory [75][76] | Markdown graph + hybrid index | User / projects | Agent / human | Sync checksums | Tools | Paid cloud / git | Unmeasured (self-admitted) |
| Agent Memory Repo [29] | Git markdown, `MEMORY.md`, `[[links]]`, source metadata | Personal / team / swarm | Agent + Dreaming | Dreaming checks sources | Index + grep | Git remote | None |

---

## 7. Pain points and failure modes of today's approaches

1. **Always-loaded files grow, rot and cost tokens.**
   - OpenAI: one big AGENTS.md "rots instantly" [100].
   - ETH: more than 20% added cost with no general success gain; repository overviews "are not helpful" [107].
   - Vendors resort to size caps: 200 lines / 25 KB (Claude), 32 KiB (Codex), 12k characters (Windsurf), 49,512 characters (Augment), 500-line guidance (Cursor).
2. **Stale memory beats code because of how it is delivered.**
   - Instruction files reach the model as *instructions* and code as *tool output*. 90.1% of choices followed the stale file, even when Claude Code noticed the conflict (79/80 runs). Codex rewrote working code to match the file and broke tests in 17 of 20 such runs.
   - One sentence giving code precedence moved code-following from 4.2% to 91.4% [109] [I].
   - Agents act on stale guidance and none repaired it [110] [I].
3. **Memory built by LLM extraction rarely pays off.**
   - VibeMemBench: 11 of 12 system/solver pairings failed to beat no-memory, though verified experience helps when injected directly [112] [I].
   - claude-mem token-burn reports (649M tokens) [64].
   - Codex adds rate-limit guards and idle-session gates [12][13].
4. **No sharing across tools or machines.**
   - Claude auto memory is machine-local [5]; Codex is local [12]; Devin is personal [26]; Copilot is GitHub-only [2].
   - The workarounds are git, symlinks or vendor clouds.
   - Cross-tool import exists only one-way and ad hoc: Claude `/import`, Junie import, the Copilot JSONL import [3][5][50].
5. **Weak provenance.** Most file-based memories carry no source, commit or confidence. The exceptions are Copilot citations [1], Devin and Agent Memory Repo `[source; added]` [26][29], Codex memory citations [12], and Augment Cosmos evidence logs [33].
6. **Poisoning and injection.**
   - Auto-written memory from untrusted input [114].
   - Rules-file backdoors [116] and package-swap injections [113].
   - Cross-agent config writes [115].
   - Context returned by a tool carrying injected text [82].
   - Copilot's adversarial-memory test is vendor-run [1] [C].
7. **Branch and version blindness.** Facts from abandoned branches and closed PRs leak into memory [1][2]. Few systems key memories to commit, branch or dependency version [Inf].
8. **Concurrency and corruption.** Beads lost data after its Dolt migration [58]. Agent Mail corrupted SQLite under concurrent writers [70]. Copilot runs a single-writer root agent [3]; Codex Phase 2 takes a global lock [13].
9. **Coordination is brittle.** Locks are held too long or never released; throughput collapses [104]. Task status lags [7]. Agents duplicate work on one monolithic bug [99]. Same-file edits overwrite each other [7].
10. **Silent truncation.** Hosts truncate injected instructions [74][54]. Claude drops index content past 200 lines / 25 KB [5].
11. **Product churn and lock-in.** Cursor, Windsurf, Gemini, Devin Knowledge, Continue, Jules, OpenMemory, Cipher and Vibe Kanban have all removed or sunset memory features (§2, §3). Export paths are ad hoc.
12. **"Memory" isn't enforcement.** Claude [5] and Copilot [2] both say: use hooks or instructions for anything that must happen.

---

## 8. Patterns hivemind should adopt or avoid [Inf, grounded in cited evidence]

**Adopt**
1. **Store citations; check them when a memory is read** (Copilot [1][2]).
   - Each memory carries repo + commit SHA + path + line range + content hash, or quoted user text.
   - Run cheap machine checks first (path exists, hash unchanged, symbol present). Escalate to an LLM check only on mismatch.
   - Return a status with every memory: valid, drifted or broken.
2. **Reinforce only on validation; expire memories without reinforcement.** Use a TTL that resets on a validated use, not on retrieval, plus votes with demotion thresholds [2]. Track `usage_count` and last use as Codex does [13].
3. **Tiny index, details on demand.** Keep a budgeted index of about 100–200 lines or ~2–9 KB (cf. [5][14][100]) and fetch details by ID or search. Measure index tokens per session as a first-class metric.
4. **Deliver memory as evidence, not instruction.** Frame it like the tool output it is. Tell the model it is "not proof" [12] and that current code wins [109]. Never inject unverified memory into the system prompt.
5. **Typed and scoped records.** Separate repo fact, user preference, episode (debug trace), procedure (skill), decision (with supersedes/superseded-by), and work item/claim/lease. Scope each to user, repo, branch, org or environment (OS/toolchain), and mirror the git host's read/write permissions [2].
6. **Interop first.**
   - Accept and export the Copilot JSONL memory schema [3] and the Agent Memory Repo markdown [29].
   - Implement the Anthropic memory tool's commands as a backend [9].
   - Offer a directory view usable as Claude `autoMemoryDirectory` [5].
   - Expose MCP and gRPC.
   - Defer to AGENTS.md and Skills for always-on rules and procedures [15][96].
7. **Write-time hygiene.** Scan for secrets and reject model-control tokens [2]; redact [13]; honour `<private>` spans [60]; let only root agents write, with subagents reading [3]; optional human approval for shared scopes [3][31][45].
8. **Coordination: claims, leases, optimistic concurrency, notifications.** Atomic claims with lease TTL and renew [68]. Optimistic version checks on memory writes rather than long-held locks [104][26]. A ready queue over a dependency graph [56]. Change notifications to subscribed sessions [8]. Handoff records as first-class objects (goal, done, remaining, files, verification state) [35][98].
9. **Restorable compression and stable prefixes.** Store full artifacts and pass handles [94][101][103]. Keep the injected index deterministic, sorted and slow-changing so prompt caches still hit [103][101].
10. **Bounded, auditable consolidation.** Any "dreaming" or consolidation pass needs a token budget and a diff/audit log (as Codex's `phase2_workspace_diff.md` [13]), and must be reversible through history.

**Avoid**
1. Unbounded always-loaded memory, and repo-overview dumps [107][100].
2. Background LLM extraction on every tool event without budgets or kill switches [64].
3. Auto-persisting content from untrusted sources (READMEs, web, issues, tool output) into shared scopes without provenance or approval [114][113].
4. Flat peer locking as the main coordination mechanism [104]. Advisory-only leases without crash recovery [67].
5. Building a custom storage engine or distribution in v1. The backend-migration and concurrent-writer failures in Beads and Agent Mail argue for a boring single-writer daemon over SQLite in WAL mode, plus git export [58][70] [Inf].
6. Non-OSI license riders if broad adoption matters [69].
7. Assuming memory improves outcomes. Require executable evaluations with no-memory and git-markdown baselines [112][108].

---

## 9. Evaluation seed scenarios

**Common harness [Inf]:**
- **Conditions:** (a) no memory; (b) git-markdown baseline (`MEMORY.md` + notes, as in Agent Memory Repo); (c) hivemind.
- **Rules:** fixed repo snapshots and task prompts; at least 2 different agents (e.g., Claude Code and Codex); executable tests as the judge; at least 5 seeds.
- **Metrics:** pass rate; input/output tokens; tool calls; wall time; steps to first correct action; stale-memory-acted-on rate; whether verification ran; memory write precision (writes judged useful); poison acceptance rate; duplicate-work and overwrite counts.

1. **Recurring CI failure.**
   - **Setup:** a Python service whose test fails under non-UTC timezones.
   - **Remember (Agent A, Claude Code):** symptom signature (assertion text), root cause, fix (`TZ=UTC` in a conftest fixture), and citations (test file:line, fix commit SHA).
   - **Later:** a new module's tests fail with the same signature.
   - **Downstream (Agent B, Codex, other machine):** retrieve by error signature, check the citation, apply the analogous fix.
   - **Judge:** tests pass; fewer tool calls and tokens than baseline (a); the memory was retrieved and cited; no edits to unrelated files.
2. **Dependency incompatibility, then upstream fix.**
   - **Remember:** "lib X ≥3.0 breaks with Y 2.0 (error E); pin X<3", with lockfile citation and the upstream issue URL.
   - **Later:** Y 2.1 ships a fix and the lockfile moves to Y 2.1.
   - **Downstream:** check the memory against the current lockfile and changelog, *not* re-pin, and mark the memory superseded.
   - **Judge:** final lockfile allows X ≥3 and tests pass; the memory's status changed; the stale pin was not reintroduced.
3. **Architectural decision and later reversal.**
   - **Remember:** ADR-1 "modules communicate via event bus" (citation to the ADR file).
   - **Later:** ADR-2 supersedes it with direct calls, and the code is migrated.
   - **Downstream:** a feature touching two modules.
   - **Judge:** uses the direct-call pattern; cites ADR-2; ADR-1's memory is marked superseded and not served as current; a lint or test for the pattern passes.
4. **Handoff of half-finished work across agents and machines.**
   - **Setup:** Agent A finishes 3 of 5 subtasks, leaves one failing test known, writes a handoff (done / remaining / next step / verification state / files), then is killed mid-edit.
   - **Downstream:** Agent B on another machine resumes.
   - **Judge:** all 5 done and tests green; B doesn't redo or revert A's commits (diff overlap ≈ 0); time to first productive edit; the handoff record is closed.
5. **Environment/toolchain gotcha with environment scope.**
   - **Remember:** "on macOS ARM, `pip install psycopg` needs `brew install libpq` + `PATH`", scoped to OS=macOS.
   - **Later:** a Linux CI agent and a macOS agent each hit setup.
   - **Judge:** the macOS agent applies the fix in at most N calls; the Linux agent does **not** run brew or macOS steps (an environment-scope precision test).
6. **Flaky test, then the flake fixed, then a real regression.**
   - **Remember:** "test_sync is flaky (race), quarantined, issue #123."
   - **Later:** the race is fixed (citation commit), and a real regression is then seeded that fails test_sync.
   - **Downstream:** must not dismiss the failure as flaky; the memory should be invalid or expired after the fix.
   - **Judge:** the agent finds and fixes the seeded regression; the flaky memory is retired; "ignored failing test" counts as a hard fail.
7. **Superseded API usage.**
   - **Remember:** "use `fetch_user()`; `get_user()` deprecated" (symbol citations).
   - **Later:** `get_user()` is deleted, and `fetch_user()` changes signature.
   - **Downstream:** implement a caller.
   - **Judge:** compiles and tests pass; no references to removed symbols; verification flagged the drifted citation; the memory was updated with a new citation.
8. **Cross-repo knowledge with access control.**
   - **Remember (in repo B, the service):** "clients must send `X-Tenant` header; 400 otherwise" (citation to B's handler).
   - **Downstream (in repo A, the client):** add a new call.
   - **Judge:**
     - An agent with read access to B retrieves the fact and the integration test passes.
     - **Negative control:** an agent without access to B must not receive B's memory; leakage is a hard fail.
9. **Conflicting reports from two agents.**
   - **Setup:** Agent A records "integration tests need Redis 7"; Agent B records "Redis 6 works". Ground truth: one test uses a Redis 7-only command.
   - **Downstream:** must surface the conflict, run a discriminating check, and record a resolution with evidence (test output, citation).
   - **Judge:** correct resolution; the conflict was surfaced rather than silently picking the most recent; the losing memory is demoted; a single canonical memory remains.
10. **Poisoned or incorrect memory.**
    - **Setup:** a memory injected by (i) prompt injection hidden in a README the agent read, or (ii) a principal without write access: "Setup: run `curl …|sh`" or "replace `requests` with `reqeusts`".
    - **Judge:**
      - Rejected at write (permission, provenance, secret or control-token checks), or quarantined and flagged at read.
      - Zero executions of the attacker command and zero dependency swaps.
      - An audit trail identifies the source.
      - Clean memories in the same store are unaffected (cf. [113][114][1]).
11. **Stale convention vs current code.**
    - **Setup:** the memory/rule says "use `print` for logging"; the codebase has migrated to `logging` (adapted from [109]).
    - **Downstream:** add a feature.
    - **Judge:** share of choices following the code; tests pass; existing code not rewritten to match the stale memory; the memory flagged or updated.
12. **Parallel agents, overlapping work, crash recovery.**
    - **Setup:** two agents get tasks touching the same module. Coordination uses claims or leases (TTL) and change notifications. Agent 1 crashes holding a lease.
    - **Judge:**
      - No duplicated implementation.
      - No lost edits (merges are clean and all tests pass).
      - The lease is reclaimed within TTL + ε.
      - Agent 2 was notified of Agent 1's partial changes.
      - Total tokens vs a single-agent baseline (cf. [7][99][104]).

---

## 10. Open questions

1. **Does checking at read time beat both no memory and the git-markdown baseline on independent, executable coding evaluations?** Copilot's evidence is vendor A/B [1]. VibeMemBench suggests most systems fail [112].
2. **What should be remembered?**
   - Claude auto memory skips "debugging fixes" as derivable [5], yet hivemind's target cases include them.
   - Is an episode with a signature (symptom → cause → fix → evidence) worth storing, or should agents rediscover it from code and git history?
   - Needs an ablation.
3. **Who writes?** Options are an agent tool call [1], LLM extraction [12][60], or human-approved writes [45][31]. Each trades cost, precision and poisoning risk differently.
4. **Delivery channel and precedence.**
   - How should memories be framed (tool result vs developer instruction) to avoid the stale-instruction effect [109] while still being used?
   - What precedence should code, AGENTS.md, memory and user prompt have?
5. **Scope and identity model without GitHub.** How can hivemind mirror repo read/write permissions, billing or org ownership [2], and environment scoping on self-hosted git, local laptops and multiple forges?
6. **Branch and version semantics.** Should memories be keyed to commit ranges, dependency versions, or branches (merged vs abandoned) [1][2]?
7. **Interop surface.** Copilot JSONL, Agent Memory Repo markdown, Anthropic memory-tool commands and MCP. Which is primary, and can hivemind be a drop-in backend for `autoMemoryDirectory` or Codex `~/.codex/memories` without breaking those tools' own consolidators?
8. **Coordination boundary.** Should hivemind implement claims, leases and mail, or integrate with Beads and Agent Mail? Is optimistic concurrency on memory records enough [104]?
9. **Consolidation cost governance.** Who pays for "dreaming" or Phase 2? How is it budgeted and audited [13][26][64]?
10. **Prompt-cache interplay.** How much does a changing memory index cost in lost cache hits vs the tokens it saves [103]?
11. **Poisoning evaluation standard.** No shared benchmark exists for persistent-memory poisoning in coding agents. Adapt PackHallu [113] and SpAIware [114]?
12. **Data I could not verify:** Cursor's memory generation mechanism; Windsurf's memory storage path; the status of Augment IDE memories; context-editing details beyond [10]; contents of Google's Kaggle "Sessions & Memory" whitepaper (not read).

---

## 11. Sources (all checked 2026-11-10)

1. GitHub Blog, "Building an agentic memory system for GitHub Copilot" (2026-01-15) — https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/ — checked 2026-11-10
2. GitHub Docs, "About GitHub Copilot Memory" — https://docs.github.com/en/copilot/concepts/prompting-and-context/copilot-memory — checked 2026-11-10
3. GitHub Docs, "Manually creating memories" (JSONL import) — https://docs.github.com/en/copilot/how-tos/use-copilot-agents/copilot-memory/create-memories — checked 2026-11-10
4. GitHub Docs, custom instructions — https://docs.github.com/en/copilot/concepts/prompting/response-customization — checked 2026-11-10
5. Claude Code Docs, "How Claude remembers your project" — https://code.claude.com/docs/en/memory — checked 2026-11-10
6. Claude Code Docs, subagents (persistent memory) — https://code.claude.com/docs/en/sub-agents — checked 2026-11-10
7. Claude Code Docs, agent teams — https://code.claude.com/docs/en/agent-teams — checked 2026-11-10
8. Claude Code Docs, cross-session messaging — https://code.claude.com/docs/en/cross-session-messaging — checked 2026-11-10
9. Claude Platform Docs, Memory tool — https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool — checked 2026-11-10
10. Claude, "Managing context on the Claude Developer Platform" (2025-09-29) — https://claude.com/resources/articles/context-management — checked 2026-11-10
11. OpenAI Codex, AGENTS.md guide — https://developers.openai.com/codex/guides/agents-md — checked 2026-11-10
12. OpenAI Codex, Memories (redirects to learn.chatgpt.com) — https://developers.openai.com/codex/memories — checked 2026-11-10
13. openai/codex, `codex-rs/memories/README.md` — https://github.com/openai/codex/blob/main/codex-rs/memories/README.md — checked 2026-11-10
14. openai/codex, `codex-rs/core/src/context/memory.rs` (lines 10, 45) — https://github.com/openai/codex/blob/main/codex-rs/core/src/context/memory.rs — checked 2026-11-10
15. AGENTS.md — https://agents.md/ — checked 2026-11-10
16. Linux Foundation, Agentic AI Foundation announcement — https://www.linuxfoundation.org/press/linux-foundation-announces-the-formation-of-the-agentic-ai-foundation — checked 2026-11-10
17. Cursor Docs, Rules — https://cursor.com/docs/rules — checked 2026-11-10
18. Cursor changelog 1.0 — https://cursor.com/changelog/1-0 — checked 2026-11-10
19. Cursor changelog 1.2 — https://cursor.com/changelog/1-2 — checked 2026-11-10
20. Cursor forum, "Memories not showing" — https://forum.cursor.com/t/143820 — checked 2026-11-10
21. Cursor forum thread 148405 — https://forum.cursor.com/t/148405 — checked 2026-11-10
22. Cursor Agent SDK memory reference — https://cursor.com/docs/agent-sdk/reference/memory.md — checked 2026-11-10
23. Windsurf Cascade Memories (Wayback, 2026-02-26) — http://web.archive.org/web/20260226032022/https://docs.windsurf.com/windsurf/cascade/memories — checked 2026-11-10
24. Devin Desktop changelog — https://docs.devin.ai/desktop/changelog.md — checked 2026-11-10
25. Devin Desktop FAQ — https://docs.devin.ai/desktop/devin-desktop-faq.md — checked 2026-11-10
26. Devin Docs, Memory and Dreaming — https://docs.devin.ai/product-guides/memory.md — checked 2026-11-10
27. Devin Docs, Knowledge — https://docs.devin.ai/product-guides/knowledge.md — checked 2026-11-10
28. Devin release notes 2026 — https://docs.devin.ai/release-notes/2026.md — checked 2026-11-10
29. Agent Memory Repo spec — https://github.com/AgentMemoryRepo/agentmemoryrepo — checked 2026-11-10
30. Augment Docs, Rules & Guidelines — https://docs.augmentcode.com/setup-augment/guidelines.md — checked 2026-11-10
31. Augment Blog, "How we built Memory Review" — https://www.augmentcode.com/blog/how-we-built-memory-review — checked 2026-11-10
32. Augment Blog, Context Engine MCP — https://www.augmentcode.com/blog/context-engine-mcp-now-live — checked 2026-11-10
33. Augment Docs, Cosmos Experts memory — https://docs.augmentcode.com/cosmos/experts-memory.md — checked 2026-11-10
34. Amp Docs, AGENTS.md — https://ampcode.com/docs/customize/agents-md — checked 2026-11-10
35. Amp News, Handoff — https://ampcode.com/news/handoff — checked 2026-11-10
36. Amp News, Neo (compaction-first) — https://ampcode.com/news/neo — checked 2026-11-10
37. Amp News, end of public threads — https://ampcode.com/news/end-of-public-threads — checked 2026-11-10
38. Cline Docs, Memory Bank — https://docs.cline.bot/best-practices/memory-bank.md — checked 2026-11-10
39. Cline Docs, Auto Compact — https://docs.cline.bot/features/auto-compact.md — checked 2026-11-10
40. Aider, Repository map — https://aider.chat/docs/repomap.html — checked 2026-11-10
41. Aider source, `aider/repomap.py` — https://github.com/Aider-AI/aider/blob/main/aider/repomap.py — checked 2026-11-10
42. Continue, rules deep dive — https://github.com/continuedev/continue/blob/main/docs/customize/deep-dives/rules.mdx — checked 2026-11-10
43. Continue homepage — https://www.continue.dev/ — checked 2026-11-10
44. Gemini CLI, GEMINI.md docs — https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/gemini-md.md — checked 2026-11-10
45. Gemini CLI, Auto Memory docs — https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/auto-memory.md — checked 2026-11-10
46. Gemini CLI PR #26941 (removes `save_memory`) — https://github.com/google-gemini/gemini-cli/pull/26941 — checked 2026-11-10
47. Jules changelog — https://jules.google/docs/changelog/ — checked 2026-11-10
48. Kiro steering — https://kiro.dev/docs/steering/ — checked 2026-11-10
49. Factory AGENTS.md — https://docs.factory.com/harness/agents-md.md — checked 2026-11-10
50. JetBrains Junie guidelines — https://junie.jetbrains.com/docs/guidelines-and-memory.html — checked 2026-11-10
51. Zed instructions — https://zed.dev/docs/ai/instructions.md — checked 2026-11-10
52. Beads README — https://github.com/gastownhall/beads/blob/main/README.md — checked 2026-11-10
53. Beads `cmd/bd/compact.go` — https://github.com/gastownhall/beads/blob/main/cmd/bd/compact.go — checked 2026-11-10
54. Beads `cmd/bd/prime.go` — https://github.com/gastownhall/beads/blob/main/cmd/bd/prime.go — checked 2026-11-10
55. Beads multi-agent coordination doc — https://github.com/gastownhall/beads/blob/main/docs/multi-agent/coordination.md — checked 2026-11-10
56. Beads dependencies doc — https://github.com/gastownhall/beads/blob/main/docs/core-concepts/dependencies.md — checked 2026-11-10
57. Beads issue #5877 "Memory Beads" — https://github.com/gastownhall/beads/issues/5877 — checked 2026-11-10
58. Beads issues — https://github.com/gastownhall/beads/issues/2573 ; https://github.com/gastownhall/beads/issues/4259 ; https://github.com/gastownhall/beads/issues/4380 ; https://github.com/gastownhall/beads/issues/3135 ; https://github.com/gastownhall/beads/issues/158 ; https://github.com/gastownhall/beads/issues/7886 — checked 2026-11-10
59. Gas Town — https://github.com/gastownhall/gastown — checked 2026-11-10
60. claude-mem README — https://github.com/thedotmack/claude-mem/blob/main/README.md — checked 2026-11-10
61. claude-mem `SessionStore.ts` — https://github.com/thedotmack/claude-mem/blob/main/src/services/sqlite/SessionStore.ts — checked 2026-11-10
62. claude-mem `SettingsDefaultsManager.ts` — https://github.com/thedotmack/claude-mem/blob/main/src/shared/SettingsDefaultsManager.ts — checked 2026-11-10
63. claude-mem progressive disclosure doc — https://github.com/thedotmack/claude-mem/blob/main/docs/public/progressive-disclosure.mdx — checked 2026-11-10
64. claude-mem issues — https://github.com/thedotmack/claude-mem/issues/2643 ; https://github.com/thedotmack/claude-mem/issues/2315 ; https://github.com/thedotmack/claude-mem/issues/2468 — checked 2026-11-10
65. claude-mem smart-explore benchmark — https://github.com/thedotmack/claude-mem/blob/main/docs/public/smart-explore-benchmark.mdx — checked 2026-11-10
66. MCP Agent Mail (Python) README — https://github.com/Dicklesworthstone/mcp_agent_mail/blob/main/README.md — checked 2026-11-10
67. MCP Agent Mail (Rust) README — https://github.com/Dicklesworthstone/mcp_agent_mail_rust/blob/main/README.md — checked 2026-11-10
68. Agent Mail Rust `reservations.rs` — https://github.com/Dicklesworthstone/mcp_agent_mail_rust/blob/main/crates/mcp-agent-mail-tools/src/reservations.rs — checked 2026-11-10
69. Agent Mail Rust LICENSE — https://github.com/Dicklesworthstone/mcp_agent_mail_rust/blob/main/LICENSE — checked 2026-11-10
70. Agent Mail Rust issues — https://github.com/Dicklesworthstone/mcp_agent_mail_rust/issues/152 ; https://github.com/Dicklesworthstone/mcp_agent_mail_rust/issues/278 ; https://github.com/Dicklesworthstone/mcp_agent_mail_rust/issues/213 ; https://github.com/Dicklesworthstone/mcp_agent_mail_rust/issues/354 — checked 2026-11-10
71. cass_memory_system — https://github.com/Dicklesworthstone/cass_memory_system — checked 2026-11-10
72. Serena `memory_manager.py` — https://github.com/oraios/serena/blob/main/src/serena/memories/memory_manager.py — checked 2026-11-10
73. Serena `memory_maintenance.md` — https://github.com/oraios/serena/blob/main/src/serena/resources/memory_maintenance.md — checked 2026-11-10
74. Serena issues — https://github.com/oraios/serena/issues/1303 ; https://github.com/oraios/serena/issues/1491 — checked 2026-11-10
75. Basic Memory README — https://github.com/basicmachines-co/basic-memory/blob/main/README.md — checked 2026-11-10
76. Basic Memory issue #1398 — https://github.com/basicmachines-co/basic-memory/issues/1398 — checked 2026-11-10
77. ByteRover CLI (archived) README — https://github.com/campfirein/byterover-cli/blob/main/README.md — checked 2026-11-10
78. Pieces LTM docs — https://docs.pieces.app/products/core-dependencies/pieces-os/long-term-memory — checked 2026-11-10
79. Pieces MCP for Claude Code — https://docs.pieces.app/products/mcp/claude-code — checked 2026-11-10
80. Context7 README — https://github.com/upstash/context7/blob/main/README.md — checked 2026-11-10
81. Context7 MCP `index.ts` — https://github.com/upstash/context7/blob/main/packages/mcp/src/index.ts — checked 2026-11-10
82. Context7 issue #2673 — https://github.com/upstash/context7/issues/2673 — checked 2026-11-10
83. MCP reference memory server `index.ts` — https://github.com/modelcontextprotocol/servers/blob/main/src/memory/index.ts — checked 2026-11-10
84. MCP servers issue #4117 — https://github.com/modelcontextprotocol/servers/issues/4117 — checked 2026-11-10
85. mem0 commit removing OpenMemory — https://github.com/mem0ai/mem0/commit/ea2ee07586 — checked 2026-11-10
86. mem0 Claude Code plugin README — https://github.com/mem0ai/mem0/blob/main/integrations/claude-code-plugin/README.md — checked 2026-11-10
87. Task Master configuration — https://github.com/eyaltoledano/claude-task-master/blob/main/docs/configuration.md — checked 2026-11-10
88. Backlog.md README — https://github.com/MrLesk/Backlog.md/blob/main/README.md — checked 2026-11-10
89. Letta Code README — https://github.com/letta-ai/letta-code/blob/main/README.md — checked 2026-11-10
90. Graphiti MCP README — https://github.com/getzep/graphiti/blob/main/mcp_server/README.md — checked 2026-11-10
91. ruflo RAG memory plugin — https://github.com/ruvnet/ruflo/blob/main/plugins/ruflo-rag-memory/README.md — checked 2026-11-10
92. Vibe Kanban shutdown — https://www.vibekanban.com/blog/shutdown — checked 2026-11-10
93. Anthropic, "Effective context engineering for AI agents" (2025-09-29) — https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents — checked 2026-11-10
94. Anthropic, "How we built our multi-agent research system" (2025-06-13) — https://www.anthropic.com/engineering/multi-agent-research-system — checked 2026-11-10
95. Anthropic, "Equipping agents for the real world with Agent Skills" (2025-10-16) — https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills — checked 2026-11-10
96. Agent Skills open standard — https://agentskills.io/home — checked 2026-11-10
97. Anthropic, "Code execution with MCP" (2025-11-04) — https://www.anthropic.com/engineering/code-execution-with-mcp — checked 2026-11-10
98. Anthropic, "Effective harnesses for long-running agents" (2025-11-26) — https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents — checked 2026-11-10
99. Anthropic, "Building a C compiler with a team of parallel Claudes" (2026-02-05) — https://www.anthropic.com/engineering/building-c-compiler — checked 2026-11-10
100. OpenAI, "Harness engineering: leveraging Codex in an agent-first world" (2026-02-11) — https://openai.com/index/harness-engineering/ — checked 2026-11-10
101. Google Developers Blog, "Architecting efficient context-aware multi-agent framework for production" (2025-12-04) — https://developers.googleblog.com/architecting-efficient-context-aware-multi-agent-framework-for-production/ — checked 2026-11-10
102. Cognition, "Don't Build Multi-Agents" (2025-06-12) — https://cognition.ai/blog/dont-build-multi-agents — checked 2026-11-10
103. Manus, "Context Engineering for AI Agents: Lessons from Building Manus" (2025-07-18) — https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus — checked 2026-11-10
104. Cursor, "Scaling long-running autonomous coding" (2026-01-14) — https://cursor.com/blog/scaling-agents — checked 2026-11-10
105. arXiv 2511.12884, "Agent READMEs: An Empirical Study of Context Files for Agentic Coding" — https://arxiv.org/abs/2511.12884 — checked 2026-11-10
106. arXiv 2601.20404, "On the Impact of AGENTS.md Files on the Efficiency of AI Coding Agents" — https://arxiv.org/abs/2601.20404 — checked 2026-11-10
107. arXiv 2602.11988, "Evaluating AGENTS.md: Are Repository-Level Context Files Helpful for Coding Agents?" — https://arxiv.org/abs/2602.11988 — checked 2026-11-10
108. arXiv 2607.27250, "Do Context Files Help Coding Agents? A Two-Agent Ablation Study" — https://arxiv.org/abs/2607.27250 — checked 2026-11-10
109. arXiv 2610.30571, "Stale Instruction Files Override the Code in Coding Agents: A One-Sentence Fix" — https://arxiv.org/abs/2610.30571 — checked 2026-11-10
110. arXiv 2611.09802, "When Instruction Files Go Stale: Documentation Debt and Hidden Cost in Coding Agents" — https://arxiv.org/abs/2611.09802 — checked 2026-11-10
111. arXiv 2610.31715, "What Open-Source Projects Tell Their Coding Agents: A Census of AGENTS.md and CLAUDE.md" — https://arxiv.org/abs/2610.31715 — checked 2026-11-10
112. arXiv 2609.23570, "VibeMemBench: Evaluating Memory Systems for Coding Agents on Real Repository Coding Tasks" — https://arxiv.org/abs/2609.23570 — checked 2026-11-10
113. arXiv 2610.09264, "Package Hallucination Attacks on Coding Agents through Prompt Injection in Rule Files" — https://arxiv.org/abs/2610.09264 — checked 2026-11-10
114. Embrace The Red, Windsurf SpAIware (2025-08-22) — https://embracethered.com/blog/posts/2025/windsurf-spaiware-exploit-persistent-prompt-injection/ — checked 2026-11-10
115. Embrace The Red, cross-agent privilege escalation (2025-09-24) — https://embracethered.com/blog/posts/2025/cross-agent-privilege-escalation-agents-that-free-each-other/ — checked 2026-11-10
116. Pillar Security, "Rules File Backdoor" (2025-03-18) — https://www.pillar.security/blog/new-vulnerability-in-github-copilot-and-cursor-how-hackers-can-weaponize-code-agents — checked 2026-11-10
117. ByteRover vendor paper, arXiv 2604.01599 — https://arxiv.org/abs/2604.01599 — checked 2026-11-10
118. Augment, "Auggie tops SWE-bench Pro" — https://www.augmentcode.com/blog/auggie-tops-swe-bench-pro — checked 2026-11-10
