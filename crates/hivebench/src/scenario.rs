//! Scenario format: timeline, gold memories, probes, distractors.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioMeta {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub family: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub actor: String,
    pub summary: String,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldClaim {
    /// Stable id used by probes (e.g. `gold-01-fix`).
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub statement: String,
    #[serde(default)]
    pub about: Vec<String>,
    #[serde(default)]
    pub when: BTreeMap<String, String>,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// active (default), superseded, retracted, invalidated
    #[serde(default = "active")]
    pub status: String,
}

fn active() -> String {
    "active".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProbeExpect {
    #[serde(default)]
    pub claim_ids: Vec<String>,
    #[serde(default)]
    pub abstain: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Probe {
    pub id: String,
    pub query: String,
    #[serde(default)]
    pub ctx: BTreeMap<String, String>,
    #[serde(default)]
    pub expect: ProbeExpect,
    #[serde(default)]
    pub include_history: bool,
    #[serde(default)]
    pub include_inapplicable: bool,
    #[serde(default)]
    pub k: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub root: PathBuf,
    pub meta: ScenarioMeta,
    pub timeline: Vec<TimelineEvent>,
    pub gold: Vec<GoldClaim>,
    pub distractors: Vec<GoldClaim>,
    pub probes: Vec<Probe>,
}

pub fn load_scenario(dir: impl AsRef<Path>) -> Result<Scenario> {
    let root = dir.as_ref().to_path_buf();
    if !root.is_dir() {
        bail!("{} is not a scenario directory", root.display());
    }
    let id = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());
    let readme = root.join("README.md");
    let title = if readme.exists() {
        first_heading(&fs::read_to_string(&readme)?).unwrap_or_else(|| id.clone())
    } else {
        id.clone()
    };
    let timeline: Vec<TimelineEvent> = read_yaml(&root.join("timeline.yaml"))?;
    let gold: Vec<GoldClaim> = read_yaml(&root.join("memories.gold.yaml"))?;
    let distractors: Vec<GoldClaim> = if root.join("distractors.yaml").exists() {
        read_yaml(&root.join("distractors.yaml"))?
    } else {
        Vec::new()
    };
    let probes: Vec<Probe> = read_yaml(&root.join("probes.yaml"))?;
    if gold.is_empty() && probes.iter().any(|p| !p.expect.abstain) {
        bail!(
            "{}: gold memories empty but probes expect hits",
            root.display()
        );
    }
    Ok(Scenario {
        meta: ScenarioMeta {
            id,
            title,
            family: String::new(),
        },
        root,
        timeline,
        gold,
        distractors,
        probes,
    })
}

pub fn load_scenarios(root: impl AsRef<Path>) -> Result<Vec<Scenario>> {
    let root = root.as_ref();
    let mut dirs: Vec<_> = fs::read_dir(root)
        .with_context(|| format!("read {}", root.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("probes.yaml").exists())
        .collect();
    dirs.sort();
    dirs.into_iter().map(load_scenario).collect()
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn first_heading(md: &str) -> Option<String> {
    md.lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim).map(str::to_owned))
}

pub fn validate_scenario(s: &Scenario) -> Result<()> {
    let gold_ids: std::collections::BTreeSet<_> = s.gold.iter().map(|c| c.id.as_str()).collect();
    if s.probes.is_empty() {
        bail!("{}: no probes", s.meta.id);
    }
    for probe in &s.probes {
        if probe.expect.abstain && !probe.expect.claim_ids.is_empty() {
            bail!(
                "{} probe {}: abstain and claim_ids both set",
                s.meta.id,
                probe.id
            );
        }
        if !probe.expect.abstain && probe.expect.claim_ids.is_empty() {
            bail!(
                "{} probe {}: expected claim_ids or abstain",
                s.meta.id,
                probe.id
            );
        }
        for id in &probe.expect.claim_ids {
            if !gold_ids.contains(id.as_str()) {
                bail!(
                    "{} probe {} expects unknown gold id `{id}`",
                    s.meta.id,
                    probe.id
                );
            }
        }
    }
    Ok(())
}
