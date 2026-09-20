# Independent MechMania sparring bot challenge

Design a standalone Rust opponent for the current MechMania engine. Return ONLY one complete Rust code block, no prose, with `use crate::core::*;` and `pub fn battle_strategy(state: &GameState, conf: &GameConfig) -> FleetAction`. No profile argument. No new crates, I/O, network, unsafe, subprocesses, or external model calls at runtime. Keep the implementation bounded (roughly 350 lines maximum) and fast under per-tick CPU limits. Add one small meaningful cfg(test) assertion if possible. You have no tools; everything needed is below.

We want an independently designed strong opponent, not a copy of our current bot. Your goals: coherent healing/targeting, rapid but supported payload pressure, and robust reinforcement behavior. A real opponent opened with 3 miners/10 fighters/4 healers and pushed to 84% of the way to our goal, then lost its healers, pushers and game. Improve on that vulnerability. Our baseline has 3 miners, recognizes healer armies, rotates wounded fighters toward medics, and can get drawn away from capture while winning attrition. We will test your executable on both sides against frozen v12; this is a test opponent and will not be submitted automatically. You do not receive baseline strategy source.

Verified mechanics: positions are mirrored by the IPC engine so our side is always bottom-left. 32 bot slots, reused after deaths. One allied bot in capture stops an enemy push regardless of counts. More bots do not increase push speed. Capture ranges -1 (our defeat) to +1 (our delivery win). Fleets die permanently only after production ends at tick max_ticks-endgame_ticks. Before then rush_order buys immediate bots while ordinary production continues. Typical config: max_ticks9000, endgame_ticks3000, HP10, speed.05, turn3 degrees/tick, shot range10, damage3, cooldown60, healing range3, heal .05 per healer per tick, stackcap3, facingarc90deg, radius.25, payloadradius.75, capture2.5. Do not hardcode these where config gives values.

Shots are hitscan after movement and turning; walls, payload, deposits, and enemy hulls block them. Friendly hulls do not block; bots do not physically collide with each other. Simultaneous shots on the same enemy only apply one damage event, so avoid redundant fire and use splash. Healing requires facing, range, walls-clear LOS, and cannot target self; friendly bodies and discs do not block healing. Walls/navigation topology is pre-initialized. `navigate_to` avoids walls only, NOT solid payload/deposit discs. Its return can be passed to move_bot; sanitize direction before predicting next position. Avoid targeting solid centers. A miner dying and self-destructing on one tick can panic the engine; avoid self-destruct.

Prefer independently chosen tactics rather than obeying this list as a prescribed implementation. Use the actual types/helpers below. `fleet_me[id]` takes u8, `action.bots[id as usize]` takes usize. Use fleet.iter() for present bots. Iterate immutable fleets; construct FleetAction only. Budget-aware expensive searches should use a bounded subset of routes.

Public core API:
//! Everything a strategy can call, in one place.
//!
//! `strategy/main.rs` does `use crate::core::*;` and gets all of this. Almost none of it is
//! defined here -- it is re-exported from the `mm-engine` crate, which is the same code the
//! engine itself runs, so the behaviour you get is the behaviour the referee gets. Follow
//! any name below into the engine source for the full doc comment.
//!
//! You never edit this file: only `src/strategy/` is yours, and only that is submitted.

#![allow(dead_code)]
#![allow(unused_imports)]

pub use mm_engine::ipc;

// ---------------------------------------------------------------------------------------
// The world you are given
// ---------------------------------------------------------------------------------------

// `GameConfig` and friends: the fixed rules of the match -- `conf.bot` (speed, health,
// turn_speed, blaster_range/damage/cooldown, heal_per_tick, extract_rate, and the `base_*`
// stats), `conf.payload` (radius, capture_radius, speed), `conf.deposit` (pos, radius,
// extractor_cap), `conf.fabricator` (interval, rush_cost, starting_tokens),
// `conf.max_ticks` / `conf.endgame_ticks`, `conf.payload_path` and `conf.map`.
// Also `conf.is_wall(x, y)`, and constants like `BOTS_MAX`, `MAP_SIZE` and `BOT_RADIUS`.
pub use mm_engine::game::config::*;
// Set by `BotChannel::handshake`, so it is available from the first tick onwards. Read the
// config instead of hardcoding numbers -- they are tuned between seasons.
pub use mm_engine::ipc::get_config;

// `Team::Me` / `Team::Other`, and `TeamPair<T>` -- indexed by team, which is what
// `state.fleets()`, `state.deposits()` and `state.fabricators()` hand back.
pub use mm_engine::game::team::*;

// ---------------------------------------------------------------------------------------
// Reading the state
// ---------------------------------------------------------------------------------------

// `GameState`: `tick`, `capture`, `fleet_me` / `fleet_other`, `deposit_me` /
// `deposit_other`, `fabricator_me` / `fabricator_other`. Useful methods:
//   state.payload_pos()             where the payload is this tick
//   state.in_endgame(conf)          are we in the last `conf.endgame_ticks`
//   state.health_pool(team)         total health of a fleet -- the first tiebreak
//
// `BotArray` (`fleet_me`): `.iter()` skips dead slots, `.get(id) -> Option<&BotState>`,
// `.is_full()`, and `[id]` indexing.
//
// `BotState`: `id`, `health`, `pos`, `vel`, `angle`, `turn_vel`, `invulnerable_until_tick`,
// plus
//   bot.class()                     Battle / Healer / Extractor
//   bot.next_fire_tick()            when this blaster is off cooldown
//   bot.shot()                      where it shot this tick, if it did
//   bot.healing()                   the bot id it is healing, if any
//   bot.extracting()                whose deposit it is mining, if any
// The last three return `StateOption<T>` (an FFI-safe Option); call `.option()` for a real
// `Option<T>`.
//
// `Deposit`: `pos`, and `extractors: TeamPair<u32>` -- a bitmask, bit `i` meaning bot `i`
// holds one of the `conf.deposit.extractor_cap` slots.
// `FabricatorState`: `tokens`, and `next_bot_creation` (an absolute tick).
pub use mm_engine::game::state::*;

// ---------------------------------------------------------------------------------------
// Geometry and angles
// ---------------------------------------------------------------------------------------

// `Vec2`: `new`, `ZERO`, `dist`, `dist_sq`, `norm`, `norm_sq`, `dot`, `normalize_or_zero`,
// `rotate_deg` / `rotate_rad`, `angle_deg` / `angle_rad`, `from_angle_deg` /
// `from_angle_rad`, and the full `+ - * /` operator set.
// Plus `normalize_degrees(deg)` (wrap into 0..360) and `diff_degrees(a, b)` (the signed
// shortest rotation between two headings -- what you want for aiming).
pub use mm_engine::game::util::*;

// ---------------------------------------------------------------------------------------
// Navigation -- all of these take `get_config()` as their first argument
// ---------------------------------------------------------------------------------------

pub use mm_engine::game::topology::{
    // Could a bot (with its radius) walk the straight line a -> b?
    corridor_clear,
    // Is a disc of this radius clear of walls?
    disc_free,
    // Zero-radius sightline: what the blaster and the extractor ray actually check.
    has_line_of_sight,
    // The navigation graph itself, if you want to build on it. `init_topology` is already
    // called for you at handshake.
    init_topology,
    // One tick's move delta from `from` towards `to`, around walls. This is a step, not a
    // plan: call it every tick and it re-routes itself. It never fails.
    navigate_to,
    // Walking distance around walls. `None` when there is no route at all.
    path_length,
    // Can a bot stand centred here? (`disc_free` at the bot radius.)
    point_free,
    // Distance from a point to a segment. Takes no config.
    point_seg_dist,
    // The corners of that route, both endpoints excluded. Allocates -- prefer the two
    // above in a hot loop.
    route_waypoints,
    topology,
    MapTopology,
};

// ---------------------------------------------------------------------------------------
// Your compute budget
// ---------------------------------------------------------------------------------------

// What this bot has left to spend, in engine ticks -- gate an expensive search on
// `get_budget().remaining` rather than being sat out for the ticks an overspend costs.
// `remaining` can go negative: an overspend is a debt, repaid in ticks you do not get to
// act on. `Budget::BANK` and `Budget::REFILL` are the bank size and the per-tick refill.
pub use mm_engine::ipc::{get_budget, Budget};

// ---------------------------------------------------------------------------------------
// What you give back
// ---------------------------------------------------------------------------------------

// `FleetAction::new()`, then per bot `action.bots[id]` (a `BotAction`: `move_action`,
// `turn_action`, `special_action`, `self_destruct`), plus fleet-wide `fabricator_next`
// (which class to build) and `rush_order` (pay `conf.fabricator.rush_cost` to build now).
//
// `MoveAction` is a direction -- anything longer than 1 is normalized for you.
// `TurnAction` is `Direction { power }`, `TargetRotation { deg }` or `TargetPosition { pos }`.
// `SpecialAction` is `Battle { fire }`, `Healer { fire, target }` or `Extractor { mine }`,
// and it is what decides the class a bot acts as.
//
// (All of the above come in with `game::state::*`, above.)

/// What `get_strategy` hands back: one function from the world to a fleet's orders,
/// called once per tick.
pub type Strategy = Box<dyn Fn(&GameState) -> FleetAction>;

// Helpful wrapper functions to make the syntax more friendly

#[inline(always)]
pub fn move_bot(vel: Vec2) -> MoveAction {
    MoveAction { direction: vel }
}
#[inline(always)]
pub fn turn_to_angle(target_angle: f32) -> TurnAction {
    TurnAction::TargetRotation { deg: target_angle }
}

#[inline(always)]
pub fn turn_towards(target_position: Vec2) -> TurnAction {
    TurnAction::TargetPosition {
        pos: target_position,
    }
}
