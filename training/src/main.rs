#[path = "../../src/core.rs"]
mod core;
mod fable;
mod fable_counters;
mod grok;
mod grok_counters;
mod local_counters;
mod strategy;

use anyhow::{bail, ensure, Context};
use serde::Deserialize;
use std::{collections::HashSet, path::Path};

const ROSTER: &str = include_str!("../roster.json");

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    name: String,
    category: String,
    description: String,
    inspired_by: Option<String>,
    evidence: Vec<String>,
    miners: usize,
    healers: usize,
    heal_every: f32,
    fighter_floor: usize,
    guards: usize,
    spacing: f32,
    range: f32,
    leash: f32,
    delay_push: u32,
    switch_tick: u32,
    late_miners: Option<usize>,
    strafe: bool,
    targeting: String,
    #[serde(default)]
    raid: bool,
    #[serde(default)]
    healing_ball: bool,
    #[serde(default)]
    mobilize: bool,
    #[serde(default)]
    opening_healers: Option<usize>,
    #[serde(default)]
    staged_push: bool,
    #[serde(default)]
    coordinated: bool,
    #[serde(default)]
    medic_leash: f32,
    #[serde(default)]
    finish_push: bool,
    #[serde(default)]
    evade_miners: bool,
}

fn roster() -> anyhow::Result<Vec<Profile>> {
    let profiles: Vec<Profile> = serde_json::from_str(ROSTER)?;
    let mut names = HashSet::new();
    ensure!(!profiles.is_empty(), "empty roster");
    for p in &profiles {
        ensure!(
            !p.name.is_empty()
                && p.name
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "invalid profile name"
        );
        ensure!(names.insert(&p.name), "duplicate profile {}", p.name);
        ensure!(
            !p.category.is_empty() && !p.description.is_empty(),
            "missing profile metadata"
        );
        ensure!(
            p.inspired_by.is_none() || !p.evidence.is_empty(),
            "inspired profiles need evidence"
        );
        ensure!(
            !p.mobilize || p.late_miners == Some(0),
            "mobilization requires zero late miners"
        );
        ensure!(
            p.miners <= 8
                && p.late_miners.unwrap_or(0) <= 8
                && p.healers <= 32
                && p.opening_healers.unwrap_or(0) <= p.healers
                && p.fighter_floor <= 32
                && p.guards <= 32,
            "invalid fleet targets: {}",
            p.name
        );
        ensure!(
            (0.0..=32.0).contains(&p.heal_every)
                && (0.0..=10.0).contains(&p.spacing)
                && (0.1..=1.0).contains(&p.range)
                && (0.0..=10.0).contains(&p.medic_leash)
                && (0.0..=64.0).contains(&p.leash),
            "invalid tactics: {}",
            p.name
        );
        ensure!(
            ["nearest", "healers", "miners", "weakest", "splash"].contains(&p.targeting.as_str()),
            "unknown targeting: {}",
            p.name
        );
    }
    Ok(profiles)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let profiles = roster()?;
    if args.get(1).map(String::as_str) == Some("--list") {
        println!("{ROSTER}");
        return Ok(());
    }
    // Engine forwards argv[0] unchanged, so each symlink selects one frozen profile.
    let name = Path::new(&args[0])
        .file_name()
        .context("missing executable name")?
        .to_str()
        .context("non-UTF8 profile name")?;
    let Some(profile) = profiles.iter().find(|p| p.name == name) else {
        bail!("unknown profile {name}; use tools/train.py build to create named executables");
    };
    ensure!(args.len() == 2, "expected engine shared-memory path");
    let channel = core::ipc::BotChannel::from_path(Path::new(&args[1]).to_path_buf())?;
    let (_, config) = channel.handshake().await?;
    let counter = local_counters::get_strategy(name)
        .or_else(|| grok_counters::get_strategy(name))
        .or_else(|| fable_counters::get_strategy(name));
    while let Some(state) = channel.await_tick().await {
        channel.respond(if let Some(strategy) = &counter {
            strategy(state)
        } else if profile.name == "fable-independent" {
            fable::battle_strategy(state, &config)
        } else if profile.name == "grok-independent" {
            grok::battle_strategy(state, &config)
        } else {
            strategy::battle_strategy(state, &config, profile)
        });
    }
    Ok(())
}
