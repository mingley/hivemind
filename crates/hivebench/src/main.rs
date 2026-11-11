//! `hivebench`: list / validate scenarios and run offline E-R.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand, ValueEnum};
use hivebench::baselines::Condition;
use hivebench::harness;
use hivebench::scenario::{self, load_scenario, load_scenarios, validate_scenario};

#[derive(Parser, Debug)]
#[command(name = "hivebench", version, about = "hivemind evaluation harness")]
struct Cli {
    /// Root directory containing scenario folders.
    #[arg(long, global = true, default_value = "scenarios")]
    scenarios: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List scenario ids.
    List,
    /// Validate scenario YAML against the schema contract.
    Validate {
        /// One scenario id, or all if omitted.
        id: Option<String>,
    },
    /// Offline retrieval (E-R) for one or all scenarios.
    Er {
        #[arg(long, value_enum, default_value_t = Cond::Hivemind)]
        condition: Cond,
        #[arg(long, default_value_t = 8)]
        k: usize,
        /// One scenario id, or all if omitted.
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Dry-run harness: E-R plus token ledger stub.
    DryRun {
        #[arg(long, value_enum, default_value_t = Cond::Hivemind)]
        condition: Cond,
        #[arg(long, default_value_t = 8)]
        k: usize,
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Cond {
    B0,
    B1b,
    B1c,
    B2,
    Oracle,
    Hivemind,
}

impl From<Cond> for Condition {
    fn from(c: Cond) -> Self {
        match c {
            Cond::B0 => Condition::B0,
            Cond::B1b => Condition::B1b,
            Cond::B1c => Condition::B1c,
            Cond::Oracle => Condition::Oracle,
            Cond::B2 => Condition::B2,
            Cond::Hivemind => Condition::Hivemind,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => {
            for s in load_scenarios(&cli.scenarios)? {
                println!("{}\t{}", s.meta.id, s.meta.title);
            }
        }
        Command::Validate { id } => {
            for s in select(&cli.scenarios, id.as_deref())? {
                validate_scenario(&s)?;
                println!("ok {}", s.meta.id);
            }
        }
        Command::Er {
            condition,
            k,
            id,
            json,
        } => {
            let condition = Condition::from(condition);
            for s in select(&cli.scenarios, id.as_deref())? {
                let m = hivebench::run_er(&s, condition, k)?;
                if json {
                    println!("{}", serde_json::to_string(&m)?);
                } else {
                    println!(
                        "{} [{}] probes={} recall@k={:.2} stale={:.2} abstain={:.2} appl={:.2}",
                        m.scenario,
                        m.condition,
                        m.probes,
                        m.recall_at_k(),
                        m.stale_hit_rate(),
                        m.abstention_accuracy(),
                        m.applicability_precision(),
                    );
                }
            }
        }
        Command::DryRun {
            condition,
            k,
            id,
            json,
        } => {
            let condition = Condition::from(condition);
            for s in select(&cli.scenarios, id.as_deref())? {
                let run = harness::dry_run(&s, condition, k)?;
                if json {
                    println!("{}", serde_json::to_string(&run)?);
                } else {
                    println!(
                        "{} [{}] recall@k={:.2} ledger_tokens={} memory_tokens={}",
                        run.scenario,
                        run.condition,
                        run.er.recall_at_k(),
                        run.ledger.total_non_judge_tokens(),
                        run.ledger.memory_attributable_tokens(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn select(root: &PathBuf, id: Option<&str>) -> Result<Vec<scenario::Scenario>> {
    match id {
        Some(id) => {
            let path = root.join(id);
            let s = load_scenario(&path).with_context(|| format!("load {}", path.display()))?;
            Ok(vec![s])
        }
        None => load_scenarios(root),
    }
}
