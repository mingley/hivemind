//! `hivemind`: command-line access to a hivemind store.
//!
//! Prefers a running `hivemindd` when present; otherwise opens the store
//! directly. `hivemind mcp` is the stdio MCP shim (M1).

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use hivemind_core::applicability::Context;
use hivemind_core::{
    Actor, ActorKind, BlobHash, Citation, ClaimData, ClaimId, ClaimKind, EdgeKind, EvidenceRef,
    Method, Origin, Outcome, Provenance, RetractReason, Scope, Timestamp,
};
use hivemind_engine::{ClaimDetail, Query, Recall, Receipt, Remember, Store, render_card};
use serde::Serialize;

#[derive(Parser, Debug)]
#[command(
    name = "hivemind",
    version,
    about = "A collective, durable memory for AI agents"
)]
struct Cli {
    /// Store directory [default: ~/.hivemind/store].
    #[arg(long, env = "HIVEMIND_DIR", global = true)]
    dir: Option<PathBuf>,
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Principal recorded as the author of writes [default: $USER].
    #[arg(
        long = "as",
        env = "HIVEMIND_PRINCIPAL",
        global = true,
        value_name = "PRINCIPAL"
    )]
    principal: Option<String>,
    /// Agent name recorded with writes, such as claude-code.
    #[arg(long, env = "HIVEMIND_AGENT", global = true)]
    agent: Option<String>,
    /// Session recorded with writes.
    #[arg(long, env = "HIVEMIND_SESSION", global = true)]
    session: Option<String>,
    /// Record writes as made by a human. Not authenticated in local mode.
    #[arg(long, global = true)]
    human: bool,
    /// Skip fsync. For tests and benchmarks only: writes survive a crash of
    /// this process but not power loss.
    #[arg(long, hide = true, global = true)]
    no_fsync: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create the store if needed and print its identity.
    Init,
    /// Record a claim.
    Remember(RememberArgs),
    /// Search memories; prints compact cards.
    Recall(RecallArgs),
    /// Show one claim in full: provenance, lifecycle, belief, and links.
    Show(ShowArgs),
    /// Mark a claim as wrong, misrecorded, or a duplicate.
    Retract(RetractArgs),
    /// Record that a claim stopped being true.
    Invalidate(InvalidateArgs),
    /// Relate two claims.
    Link(LinkArgs),
    /// Report how a claim worked out.
    Feedback(FeedbackArgs),
    /// Store a file (or `-` for stdin) as an artifact and print its hash.
    Put(PutArgs),
    /// Print an artifact's bytes.
    Cat {
        /// Artifact hash, as printed by `put`.
        hash: BlobHash,
    },
    /// Print events from the log as JSON lines (the change feed).
    Log(LogArgs),
    /// Print store statistics.
    Stats,
    /// Re-read the log and every blob, checking checksums and references.
    Verify,
    /// Run the MCP stdio shim (finds or starts hivemindd).
    Mcp {
        /// Path to hivemindd [default: sibling of this binary, or PATH].
        #[arg(long)]
        daemon: Option<PathBuf>,
    },
    /// Run the store daemon in the foreground (same as `hivemindd`).
    Daemon(hivemind::daemon::DaemonArgs),
    /// Append claims in a loop, printing each acknowledged offset (crash testing).
    #[command(hide = true)]
    DebugWriteLoop {
        #[arg(long)]
        count: Option<u64>,
    },
}

#[derive(Args, Debug)]
struct RememberArgs {
    /// The claim: one or two sentences written for a future reader.
    statement: String,
    /// fact, decision, preference, procedure, solution, warning, question, or handoff.
    #[arg(long, short, default_value = "fact")]
    kind: ClaimKind,
    /// Owning scope, such as project:github.com/acme/api.
    #[arg(long, short)]
    scope: Scope,
    /// Entity the claim is about, such as pkg:cargo/openssl-sys (repeatable).
    #[arg(long = "about", value_name = "ENTITY")]
    about: Vec<String>,
    /// Condition under which the claim holds, such as os=macos or
    /// dep:cargo/serde=^1.0 (repeatable).
    #[arg(long = "when", value_name = "KEY=VALUE", value_parser = parse_pair)]
    when: Vec<(String, String)>,
    /// Structured field, such as fix='export OPENSSL_DIR=…' (repeatable).
    #[arg(long = "field", value_name = "KEY=VALUE", value_parser = parse_pair)]
    fields: Vec<(String, String)>,
    /// Code citation: path[:start[-end]][@commit] (repeatable).
    #[arg(long = "cite", value_name = "CITATION")]
    cites: Vec<Citation>,
    #[command(flatten)]
    evidence: EvidenceArgs,
    /// Where the content came from: agent, human, tool_output, repo_file, or external.
    #[arg(long, default_value = "agent")]
    origin: Origin,
    /// How the claim was produced [default: agent_authored, or human with --human].
    #[arg(long)]
    method: Option<Method>,
    /// Active claim this one replaces: ID or unique prefix (repeatable).
    #[arg(long = "supersedes", value_name = "CLAIM")]
    supersedes: Vec<String>,
    /// Claim this one was derived from (repeatable).
    #[arg(long = "derived-from", value_name = "CLAIM")]
    derived_from: Vec<String>,
    /// When the claim became true: RFC 3339 or a date.
    #[arg(long, value_name = "TIME")]
    valid_from: Option<String>,
    /// Flag the claim as expired if not confirmed by then: a date or a duration such as 90d.
    #[arg(long, value_name = "TIME")]
    review_by: Option<String>,
    /// Retry key: repeating it returns the original result instead of writing again.
    #[arg(long)]
    idempotency_key: Option<String>,
    /// Hash `--cite` paths under this workspace (fills content_hash for drift checks).
    #[arg(long, value_name = "DIR")]
    workspace: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
struct EvidenceArgs {
    /// File to store as an artifact and cite as evidence (repeatable).
    #[arg(long = "evidence-file", value_name = "PATH")]
    files: Vec<PathBuf>,
    /// Evidence URL (repeatable).
    #[arg(long = "evidence-url", value_name = "URL")]
    urls: Vec<String>,
    /// Short verbatim evidence, such as one error line (repeatable).
    #[arg(long = "quote", value_name = "TEXT")]
    quotes: Vec<String>,
}

#[derive(Args, Debug)]
struct ContextArgs {
    /// Context for applicability, such as os=linux or dep:cargo/tokio=1.47.1 (repeatable).
    #[arg(long = "ctx", value_name = "KEY=VALUE", value_parser = parse_pair)]
    ctx: Vec<(String, String)>,
    /// Add this machine's os/arch and dependency versions from lockfiles.
    #[arg(long)]
    here: bool,
    /// Workspace root for lockfile fingerprinting and citation checks [default: cwd].
    #[arg(long, value_name = "DIR")]
    workspace: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct RecallArgs {
    /// What to look for: error text, identifiers, or a question.
    query: Vec<String>,
    /// Scope to search (repeatable) [default: all].
    #[arg(long = "scope")]
    scopes: Vec<Scope>,
    /// Kind to include (repeatable) [default: all].
    #[arg(long = "kind")]
    kinds: Vec<ClaimKind>,
    /// Only claims about this entity (repeatable).
    #[arg(long = "about", value_name = "ENTITY")]
    about: Vec<String>,
    #[command(flatten)]
    context: ContextArgs,
    /// Include superseded, invalidated, and retracted claims.
    #[arg(long)]
    history: bool,
    /// Include claims whose conditions don't match the context.
    #[arg(long)]
    all_contexts: bool,
    /// Most cards to return.
    #[arg(long)]
    limit: Option<usize>,
    /// Token budget for the returned cards.
    #[arg(long)]
    budget: Option<usize>,
}

#[derive(Args, Debug)]
struct ShowArgs {
    /// Claim ID or unique prefix.
    id: String,
    #[command(flatten)]
    context: ContextArgs,
    /// Include the content of evidence artifacts.
    #[arg(long)]
    evidence: bool,
}

#[derive(Args, Debug)]
struct RetractArgs {
    /// Claim ID or unique prefix.
    id: String,
    /// wrong, misrecorded, or duplicate.
    #[arg(long, default_value = "wrong")]
    reason: RetractReason,
    #[arg(long, default_value = "")]
    note: String,
    /// The claim that corrects this one.
    #[arg(long, value_name = "CLAIM")]
    corrected_by: Option<String>,
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Args, Debug)]
struct InvalidateArgs {
    /// Claim ID or unique prefix.
    id: String,
    /// Why the claim stopped being true, such as "fixed in commit 9e1f".
    #[arg(long)]
    reason: String,
    /// When it stopped being true [default: now].
    #[arg(long, value_name = "TIME")]
    at: Option<String>,
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Args, Debug)]
struct LinkArgs {
    /// Source claim.
    from: String,
    /// Target claim.
    to: String,
    /// supports, contradicts, refines, related, or depends_on.
    #[arg(long)]
    kind: EdgeKind,
    #[arg(long)]
    note: Option<String>,
    #[command(flatten)]
    evidence: EvidenceArgs,
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Args, Debug)]
struct FeedbackArgs {
    /// Claim ID or unique prefix.
    id: String,
    /// helpful, unhelpful, harmful, or not_applicable.
    outcome: Outcome,
    #[arg(long)]
    note: Option<String>,
    #[command(flatten)]
    evidence: EvidenceArgs,
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Args, Debug)]
struct PutArgs {
    /// File to store, or `-` for stdin.
    file: PathBuf,
    #[arg(long)]
    media_type: Option<String>,
    /// Name to record [default: the file name].
    #[arg(long)]
    name: Option<String>,
    /// Where the content came from.
    #[arg(long, default_value = "tool_output")]
    origin: Origin,
}

#[derive(Args, Debug)]
struct LogArgs {
    /// First offset to print.
    #[arg(long, default_value_t = 0)]
    from: u64,
    #[arg(long, default_value_t = 100)]
    limit: usize,
}

fn parse_pair(text: &str) -> Result<(String, String), String> {
    let (key, value) = text
        .split_once('=')
        .ok_or_else(|| format!("expected KEY=VALUE, got `{text}`"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("empty key in `{text}`"));
    }
    Ok((key.to_owned(), value.trim().to_owned()))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let dir = match &cli.dir {
        Some(dir) => dir.clone(),
        None => default_dir()?,
    };

    match &cli.command {
        Command::Mcp { daemon } => {
            return hivemind::mcp::main_sync(dir, daemon.clone());
        }
        Command::Daemon(args) => {
            let mut args = args.clone();
            if args.dir.is_none() {
                args.dir = Some(dir);
            }
            if cli.no_fsync {
                args.no_fsync = true;
            }
            return hivemind::daemon::run(args);
        }
        _ => {}
    }

    let mut backend = hivemind::backend::Backend::open(&dir, cli.no_fsync)
        .with_context(|| format!("opening store at {}", dir.display()))?;
    if let Some(store) = backend.store() {
        let recovery = store.recovery();
        if recovery.truncated_bytes > 0 {
            eprintln!(
                "warning: removed {} bytes of a torn, unacknowledged write from the end of the log; recorded as a `recovered` event",
                recovery.truncated_bytes
            );
        }
    }
    let actor = actor(&cli);
    let out = Output { json: cli.json };

    // Prefer the daemon for the core memory commands when it is running.
    if backend.is_daemon() {
        return run_via_daemon(&mut backend, &cli, actor, out);
    }

    let store = backend.store().expect("local backend always has a store");

    match cli.command {
        Command::Init => {
            let manifest = store.manifest();
            if out.json {
                out.print_json(manifest)
            } else {
                println!(
                    "store {} at {} (created {})",
                    manifest.store_id,
                    store.dir().display(),
                    manifest.created_at
                );
                Ok(())
            }
        }
        Command::Remember(args) => remember(store, &actor, args, &out),
        Command::Recall(args) => {
            let query = Query {
                text: args.query.join(" "),
                min_offset: None,
                scopes: args.scopes,
                kinds: args.kinds,
                about: args.about,
                context: context(&args.context),
                include_history: args.history,
                include_inapplicable: args.all_contexts,
                limit: args.limit,
                budget_tokens: args.budget,
                now: None,
            };
            let result = store.recall(&query);
            if out.json {
                out.print_json(&result)
            } else {
                print_recall(&result);
                Ok(())
            }
        }
        Command::Show(args) => {
            let id = store.resolve(&args.id)?;
            let ctx = context(&args.context);
            let workspace = workspace_root(&args.context);
            let detail = store
                .get_with_workspace(&id, (!ctx.is_empty()).then_some(&ctx), workspace.as_deref())
                .ok_or_else(|| anyhow!("claim {id} not found"))?;
            if out.json {
                out.print_json(&detail)
            } else {
                print_detail(store, &detail, args.evidence)
            }
        }
        Command::Retract(args) => {
            let id = store.resolve(&args.id)?;
            let corrected_by = args.corrected_by.map(|c| store.resolve(&c)).transpose()?;
            let receipt = store.retract(
                &actor,
                id,
                args.reason,
                &args.note,
                corrected_by,
                args.idempotency_key,
            )?;
            out.receipt(&receipt, "retracted")
        }
        Command::Invalidate(args) => {
            let id = store.resolve(&args.id)?;
            let at = args.at.as_deref().map(parse_time).transpose()?;
            let receipt = store.invalidate(&actor, id, at, &args.reason, args.idempotency_key)?;
            out.receipt(&receipt, "invalidated")
        }
        Command::Link(args) => {
            let from = store.resolve(&args.from)?;
            let to = store.resolve(&args.to)?;
            let evidence = collect_evidence(store, &actor, &args.evidence)?;
            let receipt = store.link(
                &actor,
                from,
                to,
                args.kind,
                args.note.as_deref(),
                evidence,
                args.idempotency_key,
            )?;
            out.receipt(&receipt, "linked")
        }
        Command::Feedback(args) => {
            let id = store.resolve(&args.id)?;
            let evidence = collect_evidence(store, &actor, &args.evidence)?;
            let receipt = store.feedback(
                &actor,
                id,
                args.outcome,
                args.note.as_deref(),
                evidence,
                args.idempotency_key,
            )?;
            out.receipt(&receipt, "recorded feedback")
        }
        Command::Put(args) => {
            let (bytes, default_name) = if args.file == Path::new("-") {
                let mut bytes = Vec::new();
                io::stdin().read_to_end(&mut bytes)?;
                (bytes, None)
            } else {
                let bytes = fs::read(&args.file)
                    .with_context(|| format!("reading {}", args.file.display()))?;
                let name = args
                    .file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                (bytes, name)
            };
            let (hash, receipt) = store.put_artifact(
                &actor,
                &bytes,
                args.media_type,
                args.name.or(default_name),
                args.origin,
            )?;
            if out.json {
                out.print_json(&serde_json::json!({ "hash": hash, "receipt": receipt }))
            } else {
                println!("{hash}");
                Ok(())
            }
        }
        Command::Cat { hash } => {
            io::stdout().write_all(&store.artifact(&hash)?)?;
            Ok(())
        }
        Command::Log(args) => {
            let mut stdout = io::stdout().lock();
            for (offset, event) in store.events(args.from, args.limit)? {
                let line = serde_json::json!({ "offset": offset, "event": event });
                serde_json::to_writer(&mut stdout, &line)?;
                writeln!(stdout)?;
            }
            Ok(())
        }
        Command::Stats => {
            let stats = store.stats();
            if out.json {
                return out.print_json(&stats);
            }
            println!("store      {}", stats.store_id);
            println!("events     {}", stats.events);
            println!("claims     {} {:?}", stats.claims, stats.by_status);
            println!("kinds      {:?}", stats.by_kind);
            println!("edges      {}", stats.edges);
            println!("artifacts  {}", stats.artifacts);
            println!("entities   {}", stats.entities);
            println!("terms      {}", stats.terms);
            println!("interned   {}", stats.interned_strings);
            println!(
                "log        {} segments, {} bytes",
                stats.log_segments, stats.log_bytes
            );
            for (offset, bytes) in &stats.recoveries {
                println!(
                    "recovery   removed a torn write of {bytes} bytes (event at offset {offset})"
                );
            }
            Ok(())
        }
        Command::Verify => {
            let report = store.verify()?;
            if out.json {
                out.print_json(&report)?;
            } else {
                println!(
                    "checked {} events in {} segments and {} blobs",
                    report.events, report.log_segments, report.blobs
                );
                for problem in &report.problems {
                    println!("problem: {problem}");
                }
            }
            if report.is_ok() {
                Ok(())
            } else {
                Err(anyhow!(
                    "verification found {} problems",
                    report.problems.len()
                ))
            }
        }
        Command::DebugWriteLoop { count } => debug_write_loop(store, &actor, count),
        Command::Mcp { .. } | Command::Daemon(_) => unreachable!("handled before opening store"),
    }
}

fn run_via_daemon(
    backend: &mut hivemind::backend::Backend,
    cli: &Cli,
    actor: Actor,
    out: Output,
) -> Result<()> {
    use serde_json::json;
    let actor_params = json!({
        "principal": actor.principal,
        "agent": actor.agent,
        "session": actor.session,
        "human": matches!(actor.kind, ActorKind::Human),
    });
    match &cli.command {
        Command::Init => {
            let hello = backend
                .call_daemon("hello", json!({}))?
                .expect("daemon backend");
            if out.json {
                out.print_json(&hello)
            } else {
                println!(
                    "daemon store {} (next offset {})",
                    hello["store_id"], hello["next_offset"]
                );
                Ok(())
            }
        }
        Command::Remember(args) => {
            let params = json!({
                "statement": args.statement,
                "kind": args.kind,
                "scope": args.scope,
                "about": args.about,
                "when": args.when,
                "fields": args.fields,
                "supersedes": args.supersedes,
                "idempotency_key": args.idempotency_key,
                "actor": actor_params,
            });
            let result = backend
                .call_daemon("remember", params)?
                .expect("daemon backend");
            if out.json {
                out.print_json(&result)
            } else {
                println!(
                    "remembered {} at offset {}",
                    result["claim"].as_str().unwrap_or("?"),
                    result["offset"]
                );
                Ok(())
            }
        }
        Command::Recall(args) => {
            let params = json!({
                "query": args.query.join(" "),
                "scopes": args.scopes,
                "kinds": args.kinds,
                "about": args.about,
                "ctx": args.context.ctx,
                "history": args.history,
                "all_contexts": args.all_contexts,
                "limit": args.limit,
                "budget": args.budget,
            });
            let result = backend
                .call_daemon("recall", params)?
                .expect("daemon backend");
            // Text mode prints the JSON cards; full card formatting needs a local store.
            out.print_json(&result)
        }
        Command::Show(args) => {
            let params = json!({
                "id": args.id,
                "ctx": args.context.ctx,
            });
            let result = backend
                .call_daemon("show", params)?
                .expect("daemon backend");
            if out.json {
                out.print_json(&result)
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
                Ok(())
            }
        }
        Command::Retract(args) => {
            let params = json!({
                "id": args.id,
                "reason": args.reason,
                "note": args.note,
                "corrected_by": args.corrected_by,
                "idempotency_key": args.idempotency_key,
                "actor": actor_params,
            });
            let result = backend
                .call_daemon("retract", params)?
                .expect("daemon backend");
            daemon_receipt(&out, &result, "retracted")
        }
        Command::Invalidate(args) => {
            let params = json!({
                "id": args.id,
                "reason": args.reason,
                "at": args.at,
                "idempotency_key": args.idempotency_key,
                "actor": actor_params,
            });
            let result = backend
                .call_daemon("invalidate", params)?
                .expect("daemon backend");
            daemon_receipt(&out, &result, "invalidated")
        }
        Command::Link(args) => {
            let params = json!({
                "from": args.from,
                "to": args.to,
                "kind": args.kind,
                "note": args.note,
                "idempotency_key": args.idempotency_key,
                "actor": actor_params,
            });
            let result = backend
                .call_daemon("link", params)?
                .expect("daemon backend");
            daemon_receipt(&out, &result, "linked")
        }
        Command::Feedback(args) => {
            let params = json!({
                "id": args.id,
                "outcome": args.outcome,
                "note": args.note,
                "idempotency_key": args.idempotency_key,
                "actor": actor_params,
            });
            let result = backend
                .call_daemon("feedback", params)?
                .expect("daemon backend");
            daemon_receipt(&out, &result, "recorded feedback")
        }
        Command::Log(args) => {
            let params = json!({ "from": args.from, "limit": args.limit });
            let result = backend
                .call_daemon("changes_since", params)?
                .expect("daemon backend");
            let mut stdout = io::stdout().lock();
            if let Some(events) = result["events"].as_array() {
                for event in events {
                    serde_json::to_writer(&mut stdout, event)?;
                    writeln!(stdout)?;
                }
            }
            Ok(())
        }
        Command::Stats => {
            let result = backend
                .call_daemon("stats", json!({}))?
                .expect("daemon backend");
            if out.json {
                out.print_json(&result)
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
                Ok(())
            }
        }
        Command::Put(_)
        | Command::Cat { .. }
        | Command::Verify
        | Command::DebugWriteLoop { .. } => Err(anyhow!(
            "this command needs direct store access; stop hivemindd or use a different --dir"
        )),
        Command::Mcp { .. } | Command::Daemon(_) => unreachable!("handled earlier"),
    }
}

fn daemon_receipt(out: &Output, result: &serde_json::Value, verb: &str) -> Result<()> {
    if out.json {
        out.print_json(result)
    } else {
        println!(
            "{verb} at offset {}",
            result["offset"].as_u64().unwrap_or(0)
        );
        Ok(())
    }
}

fn default_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("set HOME or pass --dir"))?;
    Ok(PathBuf::from(home).join(".hivemind").join("store"))
}

fn actor(cli: &Cli) -> Actor {
    let principal = cli
        .principal
        .clone()
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "local".to_owned());
    Actor {
        principal,
        kind: if cli.human {
            ActorKind::Human
        } else {
            ActorKind::Agent
        },
        agent: cli.agent.clone(),
        session: cli.session.clone(),
    }
}

fn workspace_root(args: &ContextArgs) -> Option<PathBuf> {
    if let Some(dir) = &args.workspace {
        return Some(dir.clone());
    }
    if args.here {
        return std::env::current_dir().ok();
    }
    None
}

fn context(args: &ContextArgs) -> Context {
    let mut ctx: Context = args.ctx.iter().cloned().collect();
    if args.here {
        ctx.entry("os".into())
            .or_insert_with(|| std::env::consts::OS.to_owned());
        ctx.entry("arch".into())
            .or_insert_with(|| std::env::consts::ARCH.to_owned());
    }
    if let Some(root) = workspace_root(args) {
        hivemind_core::merge_lockfile_fingerprint(&root, &mut ctx);
    }
    ctx
}

fn parse_time(text: &str) -> Result<Timestamp> {
    Ok(Timestamp::parse_relative(text, Timestamp::now())?)
}

/// Stores evidence files as artifacts and combines them with URLs and quotes.
fn collect_evidence(store: &Store, actor: &Actor, args: &EvidenceArgs) -> Result<Vec<EvidenceRef>> {
    let mut evidence = Vec::new();
    for path in &args.files {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let (hash, _) = store.put_artifact(actor, &bytes, None, name, Origin::ToolOutput)?;
        evidence.push(EvidenceRef::Artifact(hash));
    }
    evidence.extend(args.urls.iter().cloned().map(EvidenceRef::Url));
    evidence.extend(args.quotes.iter().cloned().map(EvidenceRef::Quote));
    Ok(evidence)
}

fn remember(store: &Store, actor: &Actor, args: RememberArgs, out: &Output) -> Result<()> {
    let now = Timestamp::now();
    let mut data = ClaimData::new(args.kind, args.statement, args.scope);
    data.about = args.about;
    data.applies_when = args.when.into_iter().collect();
    for (key, value) in args.fields {
        data.payload.insert(key, serde_json::Value::String(value));
    }
    data.valid_from = args
        .valid_from
        .as_deref()
        .map(|t| Timestamp::parse_relative(t, now))
        .transpose()?;
    data.review_by = args
        .review_by
        .as_deref()
        .map(|t| Timestamp::parse_relative(t, now))
        .transpose()?;
    let mut citations = args.cites;
    if let Some(root) = args
        .workspace
        .as_ref()
        .cloned()
        .or_else(|| std::env::var_os("HIVEMIND_WORKSPACE").map(PathBuf::from))
    {
        for citation in &mut citations {
            if citation.content_hash.is_none() {
                citation.content_hash = Some(
                    hivemind_core::hash_file_citation(&root, citation)
                        .map_err(|e| anyhow!("hashing citation {citation}: {e}"))?,
                );
            }
        }
    }
    data.provenance = Provenance {
        method: args.method.unwrap_or(if actor.kind == ActorKind::Human {
            Method::Human
        } else {
            Method::AgentAuthored
        }),
        origin: args.origin,
        evidence: collect_evidence(store, actor, &args.evidence)?,
        citations,
        derived_from: resolve_all(store, &args.derived_from)?,
    };
    let request = Remember {
        data,
        supersedes: resolve_all(store, &args.supersedes)?,
        idempotency_key: args.idempotency_key,
    };
    let receipt = store.remember(actor, request)?;
    if out.json {
        return out.print_json(&receipt);
    }
    let claim = receipt.claim.map(|c| c.to_string()).unwrap_or_default();
    if receipt.replayed {
        println!("already recorded {claim} (offset {})", receipt.offset);
    } else {
        println!("remembered {claim} (offset {})", receipt.offset);
    }
    if !receipt.similar.is_empty() {
        println!("\nsimilar claims in this scope; corroborate, supersede, or link them:");
        for card in &receipt.similar {
            println!("\n{}", render_card(card));
        }
    }
    Ok(())
}

fn resolve_all(store: &Store, ids: &[String]) -> Result<Vec<ClaimId>> {
    ids.iter()
        .map(|id| store.resolve(id).map_err(Into::into))
        .collect()
}

fn print_recall(result: &Recall) {
    if result.cards.is_empty() {
        println!("no matching memories");
        return;
    }
    for card in &result.cards {
        println!("{}\n", render_card(card));
    }
    println!(
        "{} of {} shown ({} omitted), about {} tokens, as of offset {}",
        result.cards.len(),
        result.matched,
        result.omitted,
        result.estimated_tokens,
        result.as_of_offset
    );
}

fn print_detail(store: &Store, detail: &ClaimDetail, with_evidence: bool) -> Result<()> {
    println!("{} ({}, {})", detail.id, detail.kind, detail.status);
    println!("{}\n", detail.statement);
    println!("scope        {}", detail.scope);
    println!(
        "belief       {} / trust {} ({} independent, {})",
        detail.belief.verification,
        detail.belief.trust,
        detail.belief.independent_support,
        detail.belief.policy
    );
    let agent = detail
        .author
        .agent
        .as_deref()
        .map(|a| format!(", {a}"))
        .unwrap_or_default();
    println!(
        "recorded     {} by {} ({}{agent}) at offset {}",
        detail.recorded_at, detail.author.principal, detail.author.kind, detail.offset
    );
    let taint = if detail.tainted {
        " (tainted: external content)"
    } else {
        ""
    };
    println!(
        "provenance   {} from {}{taint}",
        detail.provenance.method, detail.provenance.origin
    );
    if !detail.about.is_empty() {
        println!("about        {}", detail.about.join(", "));
    }
    if !detail.applies_when.is_empty() {
        let conditions: Vec<String> = detail
            .applies_when
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        println!("applies when {}", conditions.join(", "));
    }
    if let Some(applicability) = &detail.applicability {
        println!("applicable   {}", serde_json::to_string(applicability)?);
    }
    for (key, value) in &detail.payload {
        match value {
            serde_json::Value::String(s) => println!("{key:<12} {s}"),
            other => println!("{key:<12} {other}"),
        }
    }
    if let Some(at) = detail.valid_from {
        println!("valid from   {at}");
    }
    if let Some(at) = detail.review_by {
        println!("review by    {at}");
    }
    for citation in &detail.provenance.citations {
        println!("cites        {citation}");
    }
    for check in &detail.citation_checks {
        let detail = check.detail.as_deref().unwrap_or("");
        if detail.is_empty() {
            println!("cite check   {} {}", check.citation, check.status.as_str());
        } else {
            println!(
                "cite check   {} {} ({detail})",
                check.citation,
                check.status.as_str()
            );
        }
    }
    for evidence in &detail.provenance.evidence {
        match evidence {
            EvidenceRef::Artifact(hash) => {
                let info = store.artifact_info(hash);
                let name = info
                    .as_ref()
                    .and_then(|i| i.name.clone())
                    .unwrap_or_default();
                let size = info.map(|i| i.size).unwrap_or_default();
                println!("evidence     {hash} {name} ({size} bytes)");
                if with_evidence {
                    let bytes = store.artifact(hash)?;
                    for line in String::from_utf8_lossy(&bytes).lines().take(40) {
                        println!("  | {line}");
                    }
                }
            }
            EvidenceRef::Url(url) => println!("evidence     {url}"),
            EvidenceRef::Quote(quote) => println!("evidence     \"{quote}\""),
            EvidenceRef::Claim(claim) => println!("evidence     {claim}"),
        }
    }
    for id in &detail.supersedes {
        println!("supersedes   {id}");
    }
    if let Some(next) = detail.superseded_by {
        println!("superseded   by {next}");
    }
    if let Some(r) = &detail.retraction {
        println!(
            "retracted    {} by {} at {}: {}",
            r.reason, r.by, r.recorded_at, r.note
        );
    }
    if let Some(i) = &detail.invalidation {
        println!("invalidated  at {} by {}: {}", i.at, i.by, i.reason);
    }
    let f = &detail.feedback;
    if f.helpful + f.unhelpful + f.harmful + f.not_applicable > 0 {
        println!(
            "feedback     helpful {}, unhelpful {}, harmful {}, not applicable {}",
            f.helpful, f.unhelpful, f.harmful, f.not_applicable
        );
    }
    for edge in &detail.edges {
        println!(
            "edge         {} {} {}: {}",
            edge.direction, edge.kind, edge.other, edge.other_statement
        );
    }
    Ok(())
}

fn debug_write_loop(store: &Store, actor: &Actor, count: Option<u64>) -> Result<()> {
    let scope: Scope = "project:crash-test".parse()?;
    let mut stdout = io::stdout().lock();
    let mut i = 0u64;
    while count.is_none_or(|n| i < n) {
        let padding = "x".repeat((i % 50) as usize);
        let data = ClaimData::new(
            ClaimKind::Fact,
            format!("crash test write {i} {padding}"),
            scope.clone(),
        );
        let receipt = store.remember(actor, Remember::new(data))?;
        let claim = receipt.claim.expect("remember returns a claim");
        writeln!(stdout, "ack {} {claim}", receipt.offset)?;
        stdout.flush()?;
        i += 1;
    }
    Ok(())
}

struct Output {
    json: bool,
}

impl Output {
    fn print_json(&self, value: &impl Serialize) -> Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }

    fn receipt(&self, receipt: &Receipt, verb: &str) -> Result<()> {
        if self.json {
            return self.print_json(receipt);
        }
        if receipt.replayed {
            println!("already {verb} (offset {})", receipt.offset);
        } else {
            println!("{verb} (offset {})", receipt.offset);
        }
        Ok(())
    }
}
