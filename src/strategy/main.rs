use crate::core::*;

// Scout only this match; no network or opponent names are needed.
pub fn get_strategy(_team: u8) -> Strategy {
    let seen = std::cell::Cell::new(((0, 0), false, false));
    Box::new(move |state| {
        let (previous, early_contest, opening_support) = seen.get();
        let conf = get_config();
        let observed = observe_opponent(state, previous);
        // Read the funded opening before replacements hide its initial composition.
        let opening_support = observe_support_opening(state, observed, opening_support);
        let early_contest = early_contest || ((opening_support || observed.0 >= 7)
            && observe_early_contest(state, conf, observed, false));
        seen.set((observed, early_contest, opening_support));
        battle_strategy_with_opening(state, conf, observed, early_contest, opening_support)
    })
}

// The first 20 ticks reveal the funded opening, before later reinforcements change it.
fn observe_support_opening(state: &GameState, observed: (usize, usize), previous: bool) -> bool {
    previous || (state.tick <= 20 && observed.1 >= 4)
}

fn observe_early_contest(state: &GameState, conf: &GameConfig, observed: (usize, usize), previous: bool) -> bool {
    previous || ((observed.0 >= 7 || (observed.0 <= 3 && observed.1 >= 4)) && state.capture.abs() < 0.25
        && state.fleet_other.iter().any(|b| b.class() == BotClass::Battle
            && b.pos.dist(&state.payload_pos()) <= conf.payload.capture_radius))
}

fn observe_opponent(state: &GameState, previous: (usize, usize)) -> (usize, usize) {
    // Keep peak counts so killing their miners/healers does not erase what we learned.
    (
        previous.0.max(
            state
                .fleet_other
                .iter()
                .filter(|bot| bot.class() == BotClass::Extractor)
                .count(),
        ),
        previous.1.max(
            state
                .fleet_other
                .iter()
                .filter(|bot| bot.class() == BotClass::Healer)
                .count(),
        ),
    )
}

#[cfg(test)]
fn battle_strategy(state: &GameState, conf: &GameConfig) -> FleetAction {
    battle_strategy_for_opponent(state, conf, observe_opponent(state, (0, 0)))
}

#[cfg(test)]
fn battle_strategy_for_opponent(state: &GameState, conf: &GameConfig, observed: (usize, usize)) -> FleetAction {
    battle_strategy_with_contest(state, conf, observed, false)
}

#[cfg(test)]
fn battle_strategy_with_contest(state: &GameState, conf: &GameConfig, observed: (usize, usize), early_contest: bool) -> FleetAction {
    battle_strategy_with_opening(state, conf, observed, early_contest, true)
}

fn battle_strategy_with_opening(
    state: &GameState,
    conf: &GameConfig,
    observed: (usize, usize),
    early_contest: bool,
    opening_support: bool,
) -> FleetAction {
    // NOTE Do not worry about what side your bot is on!
    // The engine mirrors the world for you if you are on top,
    // so to you, you are always on the bottom left. Your fleet is always `fleet_me`.

    let mut action = FleetAction::new();

    let payload = state.payload_pos();

    let enemy_fighters = state
        .fleet_other
        .iter()
        .filter(|bot| bot.class() == BotClass::Battle)
        .count();
    let rush = enemy_fighters >= 8 && observed.0 <= 2 && observed.1 == 0;
    // Two medics already sustain a balanced opening; waiting for four loses the first fight.
    let support = observed.1 >= 2;
    // Field fighters while the opening is still being revealed. Match early rush pressure,
    // then use the normal economy against balanced and mining-heavy armies.
    let miner_goal = if state.tick < 8 || rush { 1 } else if state.tick > 20 && observed.0 >= 8 { 5 } else { 3 };

    // make a battle bot by default
    let mut next_bot = BotClass::Battle;

    // Slots get reused after death; rebuild toward the current composition target.
    if state
        .fleet_me
        .iter()
        .filter(|bot| bot.class() == BotClass::Extractor)
        .count()
        < miner_goal
    {
        next_bot = BotClass::Extractor;
    }

    let battles = state
        .fleet_me
        .iter()
        .filter(|bot| bot.class() == BotClass::Battle)
        .count();
    let healers = state
        .fleet_me
        .iter()
        .filter(|bot| bot.class() == BotClass::Healer)
        .count();
    // Do not spend every reinforcement on miners while the home route is under attack.
    let home_under_attack = state.fleet_other.iter().any(|enemy| {
        enemy.class() == BotClass::Battle
            && enemy.pos.dist(&state.deposit_me.pos) < conf.bot.blaster_range * 2.0
    });
    if state.tick > 200 && battles < 4 && home_under_attack {
        next_bot = BotClass::Battle;
    }
    let healer_goal = if support {
        (battles * 2 / 3).min(12)
    } else if rush {
        (battles / 6).min(2)
    } else {
        (battles / 5).min(3)
    };
    if next_bot == BotClass::Battle && healers < healer_goal {
        next_bot = BotClass::Healer;
    }

    // The deposit is a solid disc, so standing dead-center is not the mining spot. This is
    // the closest legal spot on our own edge of the ring: hull to hull with it, `+y` being
    // the side away from the map center on our half.
    //
    // You do not actually have to hug the ring -- an extractor mines anything within
    // `conf.bot.base_extract_range` that it has a sightline to (`has_line_of_sight`), and
    // only walls block that ray, not bots. Standing back is safer.
    let mining_spot = state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

    // An expanding economy needs the proven attrition response; tight light armies need mobility.
    let mobile = observed.0 <= 3 && opening_support;
    let guard = payload_guard_with_handoff(state, conf, mobile);
    let movement = |bot, goal, guard| spaced_move_with_discs(state, conf, bot, goal, guard, mobile);
    let mut covered = 0u32;
    let mut healing = [0u8; 32];

    for bot_state in state.fleet_me.iter() {
        let bot_action = &mut action.bots[bot_state.id as usize];

        if bot_state.class() == BotClass::Extractor {
            bot_action.move_action = movement(bot_state, mining_spot, false);
            bot_action.turn_action = turn_towards(state.deposit_me.pos);
            bot_action.special_action = SpecialAction::Extractor { mine: true };
            continue;
        }

        if bot_state.class() == BotClass::Healer {
            bot_action.move_action = movement(bot_state, bot_state.pos, false);
            let patient = state
                .fleet_me
                .iter()
                .filter(|ally| {
                    ally.id != bot_state.id
                        && ally.health < conf.bot.health
                        && (healing[ally.id as usize] as f32) < conf.bot.heal_stack_cap
                        && ally.pos.dist(&bot_state.pos)
                            < conf.bot.base_heal_range - conf.bot.speed * 2.0
                        && has_line_of_sight(conf, bot_state.pos, ally.pos)
                })
                // Keep early heavy-economy triage until sustained enemy healing is visible.
                // Save one-hit patients first, then prefer an ally we can face this tick.
                .min_by(|a, b| {
                    if observed.0 >= 6 && observed.1 < 4 {
                        return a.health.total_cmp(&b.health);
                    }
                    let ready = |ally: &BotState| {
                        diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle).abs()
                            <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                    };
                    (a.health > conf.bot.blaster_damage)
                        .cmp(&(b.health > conf.bot.blaster_damage))
                        .then(ready(b).cmp(&ready(a)))
                        .then(a.health.total_cmp(&b.health))
                });
            let follow = patient.or_else(|| {
                if observed.0 < 6 { return None; }
                // Idle medics may need to approach wounded fighters outside channel range.
                state.fleet_me.iter()
                    .filter(|ally| ally.class() == BotClass::Battle && ally.health <= conf.bot.health * 0.6)
                    .filter(|ally| ally.pos.dist(&bot_state.pos) <= conf.bot.blaster_range)
                    .filter_map(|ally| path_length(conf, bot_state.pos, ally.pos).map(|d| (ally, d)))
                    .min_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(ally, _)| ally)
            }).or_else(|| {
                state
                    .fleet_me
                    .iter()
                    .filter(|ally| ally.class() == BotClass::Battle)
                    .min_by(|a, b| a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload)))
            });
            if let Some(ally) = follow {
                if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                    bot_action.move_action = movement(bot_state, ally.pos, false);
                }
                if support {
                    if let Some(enemy) = state
                        .fleet_other
                        .iter()
                        .filter(|b| b.class() == BotClass::Battle)
                        .filter(|b| {
                            observed.0 < 6
                                || b.pos.dist(&ally.pos) < conf.bot.blaster_range + conf.bot.radius
                        })
                        .min_by(|a, b| {
                            a.pos
                                .dist_sq(&ally.pos)
                                .total_cmp(&b.pos.dist_sq(&ally.pos))
                        })
                    {
                        let rear = ally.pos
                            + (ally.pos - enemy.pos).normalize_or_zero()
                                * conf.bot.base_heal_range
                                * 0.6;
                        if point_free(conf, rear)
                            && has_line_of_sight(conf, rear, ally.pos)
                            && obstacles(state, conf)
                                .iter()
                                .all(|&(p, r)| rear.dist(&p) > r + conf.bot.radius)
                        {
                            bot_action.move_action =
                                movement(bot_state, rear, false);
                        }
                    }
                }
                bot_action.turn_action = turn_towards(ally.pos);
                bot_action.special_action = SpecialAction::Healer {
                    fire: patient.is_some(),
                    target: ally.id,
                };
                if patient.is_some() {
                    healing[ally.id as usize] += 1;
                }
            } else {
                bot_action.move_action = movement(bot_state, payload, false);
                bot_action.special_action = SpecialAction::Healer {
                    fire: false,
                    target: bot_state.id,
                };
            }
            continue;
        }

        // Everyone advances when there is no nearby enemy; one fighter holds capture.
        let rally = if support {
            // A depleted medic line cannot keep the whole army out of the endgame.
            let retreat_health = if state.in_endgame(conf) && healers * 3 < battles {
                0.3
            } else {
                0.6
            };
            if bot_state.health <= conf.bot.health * retreat_health {
                state
                    .fleet_me
                    .iter()
                    .filter(|ally| ally.class() == BotClass::Healer)
                    .filter_map(|ally| {
                        path_length(conf, bot_state.pos, ally.pos).map(|d| (ally, d))
                    })
                    .min_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(ally, _)| ally.pos)
            } else if early_contest && guard != Some(bot_state.id)
                && bot_state.health < conf.bot.health {
                regroup_position(state, conf, bot_state)
            } else {
                None
            }
        } else {
            regroup_position(state, conf, bot_state)
        };
        let contester = guard == Some(bot_state.id) && rally.is_none();
        bot_action.move_action =
            movement(bot_state, rally.unwrap_or(payload), contester);

        // Prefer an exposed enemy we can damage over a closer enemy behind cover.
        let enemy = state
            .fleet_other
            .iter()
            .filter(|enemy| {
                enemy.pos.dist(&payload) <= conf.bot.blaster_range + conf.payload.capture_radius
                    || enemy.pos.dist(&bot_state.pos) <= conf.bot.blaster_range
            })
            .min_by_key(|enemy| {
                let exposed = clear_shot(state, conf, bot_state.pos, enemy.pos)
                    && bot_state.pos.dist(&enemy.pos) <= conf.bot.blaster_range
                    && enemy.invulnerable_until_tick <= state.tick
                    && covered & (1 << enemy.id) == 0;
                (
                    !exposed,
                    // Attrition armies can heal or replace chip damage; finish their wounded.
                    // Keep nearest-target pressure against light economies, including healing formations.
                    if exposed && (observed.0 >= 6 || (mobile && support && early_contest)) {
                        (enemy.health * 100.0) as u32
                    } else {
                        0
                    },
                    (bot_state.pos.dist_sq(&enemy.pos) * 1000.0) as u32,
                )
            });
        let enemy = match enemy {
            Some(enemy) => enemy,
            None => continue,
        };
        let goal = rally.unwrap_or_else(|| {
            // Do not detour into a defended economy when cover blocks our chosen shot.
            if ((observed.0 >= 6
                && state.tick >= 1500
                && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range)
                || (support && observed.0 < 6 && enemy.class() == BotClass::Extractor
                    && enemy.pos.dist(&payload) > conf.payload.capture_radius))
                && !clear_shot(state, conf, bot_state.pos, enemy.pos)
            {
                return payload;
            }
            firing_position(
                state,
                conf,
                bot_state.pos,
                enemy.pos,
                contester,
                observed.0 < 6 && !support,
            )
        });
        bot_action.move_action = movement(bot_state, goal, contester);
        // Match the engine's movement clamp before predicting this tick's firing origin.
        bot_action.move_action.sanitize();
        // Hitscan has no travel time. Lead only the next movement step, before shots resolve.
        let mut aim = enemy.pos + enemy.vel;
        bot_action.turn_action = turn_towards(aim);
        let origin = bot_state.pos + bot_action.move_action.direction * conf.bot.speed;
        let desired = (aim - origin).angle_deg();
        let angle = bot_state.angle
            + diff_degrees(desired, bot_state.angle)
                .clamp(-conf.bot.turn_speed, conf.bot.turn_speed);
        let mut hits = shot_targets(state, conf, origin, angle) & !covered;
        if observed.0 < 6 && hits == 0 && bot_state.next_fire_tick() <= state.tick {
            // If the planned target needs more turning, take another legal shot this tick.
            for other in state.fleet_other.iter() {
                let candidate = other.pos + other.vel;
                let desired = (candidate - origin).angle_deg();
                let angle = bot_state.angle
                    + diff_degrees(desired, bot_state.angle)
                        .clamp(-conf.bot.turn_speed, conf.bot.turn_speed);
                let available = shot_targets(state, conf, origin, angle) & !covered;
                if available.count_ones() > hits.count_ones() {
                    aim = candidate;
                    hits = available;
                }
            }
            bot_action.turn_action = turn_towards(aim);
        }
        let fire = bot_state.next_fire_tick() <= state.tick && hits != 0;
        bot_action.special_action = SpecialAction::Battle { fire };
        if fire {
            covered |= hits; // Simultaneous hits only deal damage once per enemy.
        }
    }

    action.fabricator_next = next_bot;

    // Rush orders are the only thing tokens buy. Ask for one when we can actually pay
    // `conf.fabricator.rush_cost`, and not once the endgame has started -- no bot is built
    // in the last `conf.endgame_ticks` of the match, so the tokens would just sit there.
    action.rush_order =
        !state.in_endgame(conf) && state.fabricator_me.tokens >= conf.fabricator.rush_cost;

    action
}

// ponytail: local health is a strength estimate, not a cooldown simulation.
fn regroup_position(state: &GameState, conf: &GameConfig, bot: &BotState) -> Option<Vec2> {
    let mut enemy_health = 0.0;
    let mut enemy_center = Vec2::ZERO;
    let mut enemies = 0;
    for enemy in state
        .fleet_other
        .iter()
        .filter(|b| b.class() == BotClass::Battle)
    {
        if bot.pos.dist(&enemy.pos) < conf.bot.blaster_range {
            enemy_center += enemy.pos;
            enemy_health += enemy.health;
            enemies += 1;
        }
    }
    if enemies < 2 {
        return None;
    }
    let health: f32 = state
        .fleet_me
        .iter()
        .filter(|b| {
            b.class() == BotClass::Battle && b.pos.dist(&bot.pos) < conf.bot.blaster_range * 0.6
        })
        .map(|b| b.health)
        .sum();
    if enemy_health <= health * 1.1 && bot.health > conf.bot.health * 0.6 {
        return None;
    }
    enemy_center = enemy_center / enemies as f32;
    // Fall back within healing range of a reachable medic farther from the enemy group.
    let medic = state
        .fleet_me
        .iter()
        .filter(|b| b.class() == BotClass::Healer)
        .filter(|b| b.pos.dist(&enemy_center) > bot.pos.dist(&enemy_center))
        .filter_map(|b| path_length(conf, bot.pos, b.pos).map(|distance| (b, distance)))
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(b, _)| b)?;
    let goal =
        medic.pos + (bot.pos - medic.pos).normalize_or_zero() * conf.bot.base_heal_range * 0.5;
    if point_free(conf, goal)
        && obstacles(state, conf)
            .iter()
            .all(|&(p, r)| goal.dist(&p) > r + conf.bot.radius)
    {
        Some(goal)
    } else {
        None
    }
}

#[cfg(test)]
fn payload_guard(state: &GameState, conf: &GameConfig) -> Option<BotId> {
    payload_guard_with_handoff(state, conf, true)
}

fn payload_guard_with_handoff(state: &GameState, conf: &GameConfig, handoff: bool) -> Option<BotId> {
    let payload = state.payload_pos();
    state
        .fleet_me
        .iter()
        .filter(|bot| bot.class() == BotClass::Battle)
        .filter_map(|bot| {
            // Keep a fighter already covering capture; otherwise choose the shortest route.
            // Equal scores use slot id so minor movement inside capture does not churn roles.
            let distance = if bot.pos.dist(&payload) <= conf.payload.capture_radius {
                Some(0.0)
            } else {
                path_length(conf, bot.pos, payload)
            };
            distance.map(|distance| (bot.id, distance))
        })
        .min_by(|(a_id, a), (b_id, b)| {
            // Rotate a damaged guard before full retreat; retain a fallback if everyone is hurt.
            (handoff && state.fleet_me[*a_id].health <= conf.bot.health * 0.8)
                .cmp(&(handoff && state.fleet_me[*b_id].health <= conf.bot.health * 0.8))
                .then(a.total_cmp(b)).then(a_id.cmp(b_id))
        })
        .map(|(id, _)| id)
}

// Allies do not collide in the engine, so navigation alone piles them onto one point.
// ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
#[cfg(test)]
fn spaced_move(state: &GameState, conf: &GameConfig, bot: &BotState, goal: Vec2, guard: bool) -> MoveAction {
    spaced_move_with_discs(state, conf, bot, goal, guard, true)
}

fn spaced_move_with_discs(
    state: &GameState,
    conf: &GameConfig,
    bot: &BotState,
    goal: Vec2,
    guard: bool,
    steer_discs: bool,
) -> MoveAction {
    let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
    travel.sanitize();
    // ponytail: local disc steering can still stall between obstacles; use waypoints if measured.
    // Wall routing does not see solid payload/deposit discs. Stop at an arrival
    // ring, or slide around a disc when the destination is beyond it.
    for (center, radius) in obstacles(state, conf) {
        if !steer_discs { break; }
        let clearance = radius + conf.bot.radius;
        if (bot.pos + travel.direction * conf.bot.speed).dist(&center) >= clearance {
            continue;
        }
        if goal.dist(&center) <= clearance {
            travel.direction = Vec2::ZERO;
            break;
        }
        let desired = travel.direction;
        travel.direction = Vec2::ZERO;
        for angle in [30.0, -30.0, 60.0, -60.0, 90.0, -90.0, 120.0, -120.0] {
            let direction = desired.rotate_deg(angle);
            let next = bot.pos + direction * conf.bot.speed;
            if point_free(conf, next) && obstacles(state, conf).iter()
                .all(|&(p, r)| next.dist(&p) >= r + conf.bot.radius) {
                travel.direction = direction;
                break;
            }
        }
    }
    let spacing = 2.0 * (conf.bot.radius + conf.bot.base_blaster_splash_radius) + 0.2;
    let mut push = Vec2::new(0.0, 0.0);
    for ally in state.fleet_me.iter().filter(|ally| ally.id != bot.id) {
        let delta = bot.pos - ally.pos;
        let distance = delta.norm();
        if distance >= spacing {
            continue;
        }
        let away = if distance > EPSILON {
            delta / distance
        } else {
            // Opposite directions for an overlapping pair break perfect symmetry.
            let angle = bot.id.min(ally.id) as f32 * 137.5 + bot.id.max(ally.id) as f32 * 37.0;
            Vec2::from_angle_deg(angle) * if bot.id < ally.id { 1.0 } else { -1.0 }
        };
        push += away * ((spacing - distance) / spacing);
    }
    let strength = if state
        .fleet_other
        .iter()
        .filter(|b| b.class() == BotClass::Healer)
        .count()
        >= 4
    {
        0.5
    } else {
        3.0
    };
    let mut spread = move_bot(travel.direction + push * strength);
    spread.sanitize();
    let next = bot.pos + spread.direction * conf.bot.speed;
    let payload = state.payload_pos();
    // Keep wall navigation as fallback, and never let spacing abandon capture.
    if !point_free(conf, next)
        || obstacles(state, conf)
            .iter()
            .any(|&(p, radius)| next.dist(&p) < radius + conf.bot.radius)
        || (guard
            && bot.pos.dist(&payload) <= conf.payload.capture_radius
            && next.dist(&payload) > conf.payload.capture_radius - conf.bot.speed)
    {
        travel
    } else {
        spread
    }
}

// The engine's ray scanner is engine-only; use the same ray/circle intersection here.
fn ray_circle(origin: Vec2, dir: Vec2, center: Vec2, radius: f32) -> Option<f32> {
    let offset = center - origin;
    let along = offset.dot(dir);
    let c = offset.norm_sq() - radius * radius;
    let discriminant = along * along - c;
    if (c > 0.0 && along < 0.0) || discriminant < 0.0 {
        None
    } else {
        Some((along - discriminant.sqrt()).max(0.0))
    }
}

fn obstacles(state: &GameState, conf: &GameConfig) -> [(Vec2, f32); 3] {
    [
        (state.payload_pos(), conf.payload.radius),
        (state.deposit_me.pos, conf.deposit.radius),
        (state.deposit_other.pos, conf.deposit.radius),
    ]
}

fn clear_shot(state: &GameState, conf: &GameConfig, from: Vec2, to: Vec2) -> bool {
    has_line_of_sight(conf, from, to)
        && obstacles(state, conf).iter().all(|&(center, radius)| {
            ray_circle(from, (to - from).normalize_or_zero(), center, radius)
                .map_or(true, |distance| {
                    distance >= from.dist(&to) - conf.bot.radius
                })
        })
}

fn firing_position(
    state: &GameState,
    conf: &GameConfig,
    from: Vec2,
    target: Vec2,
    guard: bool,
    standoff: bool,
) -> Vec2 {
    let payload = state.payload_pos();
    let in_position = !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
    if in_position
        && from.dist(&target) <= conf.bot.blaster_range * if standoff { 0.95 } else { 0.85 }
        && clear_shot(state, conf, from, target)
    {
        return from;
    }
    // ponytail: eight firing positions, not a tactical search; expand only if replays show missed angles.
    let center = if guard { payload } else { target };
    let radius = if guard {
        conf.payload.radius + conf.bot.radius + 0.15
    } else {
        conf.bot.blaster_range * if standoff { 0.95 } else { 0.8 }
    };
    let base = (from - center).normalize_or_zero() * radius;
    let mut best = None;
    for step in 0..8 {
        let point = center + base.rotate_deg(step as f32 * 45.0);
        if !point_free(conf, point)
            || obstacles(state, conf)
                .iter()
                .any(|&(p, r)| point.dist(&p) < r + conf.bot.radius)
            || (guard && point.dist(&payload) > conf.payload.capture_radius - conf.bot.speed)
            || point.dist(&target) > conf.bot.blaster_range
            || !clear_shot(state, conf, point, target)
        {
            continue;
        }
        if let Some(distance) = path_length(conf, from, point) {
            if best.map_or(true, |(_, old)| distance < old) {
                best = Some((point, distance));
            }
        }
    }
    best.map_or(if guard { payload } else { target }, |(point, _)| point)
}

fn shot_targets(state: &GameState, conf: &GameConfig, origin: Vec2, angle: f32) -> u32 {
    let dir = Vec2::from_angle_deg(angle);
    let mut reach = conf.bot.blaster_range;
    for (position, direction) in [(origin.x, dir.x), (origin.y, dir.y)] {
        if direction.abs() > EPSILON {
            let edge = if direction > 0.0 {
                MAP_SIZE as f32
            } else {
                0.0
            };
            reach = reach.min(((edge - position) / direction).max(0.0));
        }
    }
    // ponytail: one-tick velocity prediction; collision shoves can still cause a miss.
    for enemy in state.fleet_other.iter() {
        if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius) {
            reach = reach.min(distance);
        }
    }
    for (center, radius) in obstacles(state, conf) {
        if let Some(distance) = ray_circle(origin, dir, center, radius) {
            reach = reach.min(distance);
        }
    }
    let impact = origin + dir * reach;
    // Conservatively skip wall impacts, even when splash might reach around the corner.
    if !has_line_of_sight(conf, origin, impact) {
        return 0;
    }
    let splash = conf.bot.base_blaster_splash_radius + conf.bot.radius;
    state
        .fleet_other
        .iter()
        .filter(|enemy| {
            enemy.invulnerable_until_tick <= state.tick
                && (enemy.pos + enemy.vel).dist(&impact) <= splash
        })
        .fold(0, |mask, enemy| mask | (1 << enemy.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combat_regression() {
        let mut conf = GameConfig {
            max_ticks: 9000,
            endgame_ticks: 3000,
            map: [[MapTile::Empty; MAP_SIZE]; MAP_SIZE],
            payload_path: PAYLOAD_PATH,
            bot: BotConfig {
                radius: 0.25,
                speed: 0.05,
                health: 10.0,
                turn_speed: 3.0,
                blaster_cooldown: 60,
                blaster_range: 10.0,
                blaster_damage: 3.0,
                base_invulnerability_ticks: 15,
                base_blaster_splash_radius: 0.3,
                heal_per_tick: 0.05,
                base_heal_range: 3.0,
                base_heal_arc_deg: 90.0,
                heal_stack_cap: 3.0,
                base_extract_range: 5.0,
                extract_rate: 0.05,
            },
            payload: PayloadConfig {
                radius: 0.75,
                capture_radius: 2.5,
                speed: 0.02,
            },
            deposit: DepositConfig {
                pos: DEPOSIT_POS,
                radius: 0.5,
                extractor_cap: 8,
            },
            fabricator: FabricatorConfig {
                interval: 200,
                rush_cost: 50.0,
                starting_tokens: 800.0,
            },
        };
        // Route-selection fixture: the direct route from the right is blocked.
        for y in 10..23 {
            conf.map[20][y] = MapTile::Wall;
        }
        init_topology(&conf.map, conf.bot.radius);
        // Heavy armies that contest early require damage control without abandoning the guard.
        let mut contested = GameState::new(&conf);
        contested.tick = 1000;
        for (pos, class, health) in [
            (Vec2::new(10.0, 10.0), BotClass::Battle, 8.0),
            (Vec2::new(7.0, 10.0), BotClass::Healer, 10.0),
            (contested.payload_pos() + Vec2::new(0.0, 1.5), BotClass::Battle, 10.0),
        ] {
            let id = contested.fleet_me.add();
            contested.fleet_me[id].pos = pos;
            contested.fleet_me[id].health = health;
            contested.fleet_me[id].special = SpecialState::new(class);
        }
        for pos in [Vec2::new(14.0, 10.0), Vec2::new(14.0, 12.0)] {
            let id = contested.fleet_other.add();
            contested.fleet_other[id].pos = pos;
            contested.fleet_other[id].health = conf.bot.health;
        }
        let retreat = battle_strategy_with_contest(&contested, &conf, (7, 9), true);
        assert!(retreat.bots[0].move_action.direction.x < 0.0,
            "damaged outnumbered fighter rejoins support after an early heavy contest");
        let guard_next = contested.fleet_me[2].pos + retreat.bots[2].move_action.direction * conf.bot.speed;
        assert!(guard_next.dist(&contested.payload_pos()) <= conf.payload.capture_radius,
            "the healthy payload guard keeps covering the objective");
        assert!(battle_strategy_with_contest(&contested, &conf, (7, 9), false)
            .bots[0].move_action.direction.x >= 0.0, "late defensive economies retain existing pressure");
        contested.fleet_me[0].health = conf.bot.health;
        assert!(battle_strategy_with_contest(&contested, &conf, (7, 9), true)
            .bots[0].move_action.direction.x >= 0.0, "healthy fighters do not retreat under the new rule");
        contested.fleet_other[0].pos = contested.payload_pos() + Vec2::new(-1.5, 0.0);
        assert!(observe_early_contest(&contested, &conf, (7, 9), false));
        assert!(observe_early_contest(&contested, &conf, (3, 4), false));
        assert!(!observe_early_contest(&contested, &conf, (3, 3), false));
        contested.fleet_me[2].health = 7.0;
        assert_eq!(payload_guard(&contested, &conf), Some(0), "rotate a damaged guard before it needs full retreat");
        assert_eq!(payload_guard_with_handoff(&contested, &conf, false), Some(2),
            "expanding economies retain the verified attrition guard policy");
        contested.fleet_me[2].health = 4.0;
        contested.fleet_me[0].health = 4.0;
        assert_eq!(payload_guard(&contested, &conf), Some(2), "retain a reachable guard when all fighters are wounded");
        contested.fleet_me[2].health = conf.bot.health;
        contested.capture = 0.5;
        contested.fleet_other[0].pos = contested.payload_pos() + Vec2::new(-1.5, 0.0);
        assert!(!observe_early_contest(&contested, &conf, (7, 9), false), "do not switch after an established push");
        assert!(observe_early_contest(&contested, &conf, (7, 9), true), "retain the learned response when the contest moves");
        contested.tick = 20;
        assert!(observe_support_opening(&contested, (3, 4), false));
        assert!(!observe_support_opening(&contested, (3, 2), false));
        contested.tick = 21;
        assert!(!observe_support_opening(&contested, (3, 4), false));
        assert!(observe_support_opening(&contested, (3, 2), true));
        let mut disc = GameState::new(&conf);
        let id = disc.fleet_me.add();
        disc.fleet_me[id].pos = disc.payload_pos() - Vec2::new(1.001, 0.0);
        let arriving = spaced_move(&disc, &conf, &disc.fleet_me[id], disc.payload_pos(), false);
        assert!(arriving.direction.norm() < EPSILON, "stop at the solid payload's capture ring");
        let passing = spaced_move(&disc, &conf, &disc.fleet_me[id], disc.payload_pos() + Vec2::new(3.0, 0.0), false);
        let next = disc.fleet_me[id].pos + passing.direction * conf.bot.speed;
        assert!(passing.direction.norm() > 0.5 && next.dist(&disc.payload_pos()) >= conf.payload.radius + conf.bot.radius,
            "steer around a disc without sending the unchecked blocked route again");

        let mut state = GameState::new(&conf);
        state.fleet_me.add();
        state.fleet_other.add();
        state.fleet_me[0].pos = Vec2::new(10.0, 16.0);
        state.fleet_other[0].pos = Vec2::new(12.0, 16.0);
        let fires = |action: &FleetAction| {
            matches!(
                action.bots[0].special_action,
                SpecialAction::Battle { fire: true }
            )
        };
        assert!(
            fires(&battle_strategy(&state, &conf)),
            "payload guard must shoot"
        );
        state.fleet_me[0].angle = 180.0;
        assert!(
            !fires(&battle_strategy(&state, &conf)),
            "wait for barrel to turn"
        );
        state.fleet_me[0].angle = 0.0;
        state.fleet_me[0].special = SpecialState::Battle {
            next_fire_tick: 10,
            shot: StateOption::None,
        };
        assert!(!fires(&battle_strategy(&state, &conf)), "respect cooldown");
        state.fleet_me[0].special = SpecialState::new(BotClass::Battle);
        let origin = state.fleet_me[0].pos;
        assert_eq!(shot_targets(&state, &conf, origin, 0.0), 1);
        state.fleet_other[0].invulnerable_until_tick = 15;
        assert_eq!(shot_targets(&state, &conf, origin, 0.0), 0);
        state.fleet_other[0].invulnerable_until_tick = 0;
        conf.map[11][16] = MapTile::Wall;
        assert_eq!(
            shot_targets(&state, &conf, origin, 0.0),
            0,
            "wall blocks shot"
        );
        assert!(!clear_shot(&state, &conf, origin, state.fleet_other[0].pos));
        conf.map[11][16] = MapTile::Empty;
        state.deposit_me.pos = Vec2::new(11.0, 16.0);
        assert_eq!(
            shot_targets(&state, &conf, origin, 0.0),
            0,
            "deposit blocks shot"
        );
        state.deposit_me.pos = DEPOSIT_POS;
        state.fleet_other[0].pos = Vec2::new(18.0, 16.0);
        assert_eq!(
            shot_targets(&state, &conf, origin, 0.0),
            0,
            "payload blocks shot"
        );
        let flank = firing_position(&state, &conf, origin, state.fleet_other[0].pos, true, true);
        assert!(flank.dist(&state.payload_pos()) < conf.payload.capture_radius);
        assert!(
            clear_shot(&state, &conf, flank, state.fleet_other[0].pos),
            "guard finds a firing angle"
        );
        // Allies are transparent to the blaster; do not invent friendly-fire blocking.
        state.fleet_other[0].pos = Vec2::new(12.0, 16.0);
        let ally = state.fleet_me.add();
        state.fleet_me[ally].pos = Vec2::new(11.0, 16.0);
        assert_eq!(shot_targets(&state, &conf, origin, 0.0), 1);
        let orders = battle_strategy(&state, &conf);
        assert!(fires(&orders));
        assert!(
            matches!(
                orders.bots[ally as usize].special_action,
                SpecialAction::Battle { fire: false }
            ),
            "do not duplicate a hit in the same tick"
        );

        state.fleet_other[0].pos = Vec2::new(31.0, 31.0);
        let orders = battle_strategy(&state, &conf);
        for bot in state.fleet_me.iter() {
            assert!(
                orders.bots[bot.id as usize]
                    .move_action
                    .direction
                    .dot(state.payload_pos() - bot.pos)
                    > 0.0,
                "advance to payload instead of chasing distant miners"
            );
        }
        state.fleet_other = GameState::new(&conf).fleet_other;
        assert!(
            battle_strategy(&state, &conf).bots[ally as usize]
                .move_action
                .direction
                .norm_sq()
                > 0.0,
            "advance without enemies too"
        );

        state.fleet_me[0].health = 4.0;
        state.fleet_me[ally].health = 1.0;
        state.fleet_me[ally].special = SpecialState::new(BotClass::Healer);
        assert!(
            matches!(
                battle_strategy(&state, &conf).bots[ally as usize].special_action,
                SpecialAction::Healer {
                    fire: true,
                    target: 0
                }
            ),
            "heal wounded ally, never self"
        );
        state.fleet_me[0].health = conf.bot.health;
        assert!(
            matches!(
                battle_strategy(&state, &conf).bots[ally as usize].special_action,
                SpecialAction::Healer { fire: false, .. }
            ),
            "healthy ally needs no healing"
        );
        state.fleet_me[0].health = 4.0;
        state.fleet_me[0].pos = Vec2::new(5.0, 16.0);
        assert!(
            matches!(
                battle_strategy(&state, &conf).bots[ally as usize].special_action,
                SpecialAction::Healer { fire: false, .. }
            ),
            "respect healing range"
        );
        state.fleet_me[0].pos = Vec2::new(13.0, 16.0);
        conf.map[12][16] = MapTile::Wall;
        assert!(
            matches!(
                battle_strategy(&state, &conf).bots[ally as usize].special_action,
                SpecialAction::Healer { fire: false, .. }
            ),
            "walls block healing"
        );
        conf.map[12][16] = MapTile::Empty;

        let mut triage = GameState::new(&conf);
        for (pos, health) in [
            (Vec2::new(10.0, 10.0), 5.0),
            (Vec2::new(12.0, 12.0), 6.0),
            (Vec2::new(12.0, 10.0), 10.0),
        ] {
            let id = triage.fleet_me.add();
            triage.fleet_me[id].pos = pos;
            triage.fleet_me[id].health = health;
        }
        triage.fleet_me[2].special = SpecialState::new(BotClass::Healer);
        triage.fleet_me[2].angle = 90.0;
        assert!(
            matches!(
                battle_strategy(&triage, &conf).bots[2].special_action,
                SpecialAction::Healer {
                    fire: true,
                    target: 1
                }
            ),
            "among noncritical patients, heal the ally already in the medic's turnable arc"
        );
        assert!(
            matches!(
                battle_strategy_for_opponent(&triage, &conf, (8, 2)).bots[2].special_action,
                SpecialAction::Healer {
                    fire: true,
                    target: 0
                }
            ),
            "heavy-economy matchups retain lowest-health patient priority"
        );
        assert!(
            matches!(
                battle_strategy_for_opponent(&triage, &conf, (8, 4)).bots[2].special_action,
                SpecialAction::Healer {
                    fire: true,
                    target: 1
                }
            ),
            "sustained enemy healing enables turn-aware triage even against a heavy economy"
        );
        triage.fleet_me[0].health = conf.bot.blaster_damage;
        assert!(
            matches!(
                battle_strategy(&triage, &conf).bots[2].special_action,
                SpecialAction::Healer {
                    fire: true,
                    target: 0
                }
            ),
            "one-shot patients take priority even when the medic needs to turn"
        );
        triage.fleet_me[0].health = 5.0;
        triage.fleet_me[2].angle = 135.0;
        assert!(
            matches!(
                battle_strategy(&triage, &conf).bots[2].special_action,
                SpecialAction::Healer {
                    fire: true,
                    target: 0
                }
            ),
            "when both patients are in the healing arc, heal the lower-health ally"
        );
        let mut heavy_support = GameState::new(&conf);
        heavy_support.tick = 20;
        for class in [
            BotClass::Extractor,
            BotClass::Extractor,
            BotClass::Extractor,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Healer,
            BotClass::Healer,
        ] {
            let id = heavy_support.fleet_me.add();
            heavy_support.fleet_me[id].special = SpecialState::new(class);
        }
        // Heavy-economy support triage is tested after funding the five-miner target.
        for _ in 0..2 {
            let id = heavy_support.fleet_me.add();
            heavy_support.fleet_me[id].special = SpecialState::new(BotClass::Extractor);
        }
        assert_eq!(
            battle_strategy_for_opponent(&heavy_support, &conf, (8, 2)).fabricator_next,
            BotClass::Healer,
            "recognize a mining-heavy healing army before it reaches four medics"
        );
        for _ in 0..8 { heavy_support.fleet_other.add(); }
        assert_eq!(battle_strategy_with_opening(&heavy_support, &conf, (3, 2), false, false).fabricator_next,
            BotClass::Healer, "recognize two medics in a balanced army before losing the first fight");
        assert_eq!(
            battle_strategy_for_opponent(&heavy_support, &conf, (8, 1)).fabricator_next,
            BotClass::Battle,
            "one observed medic alone does not trigger the healing-army response"
        );

        let mut guards = GameState::new(&conf);
        assert_eq!(payload_guard(&guards, &conf), None);
        for _ in 0..3 {
            guards.fleet_me.add();
        }
        guards.fleet_me[0].pos = Vec2::new(22.0, 16.0);
        guards.fleet_me[1].pos = Vec2::new(8.0, 16.0);
        guards.fleet_me[2].pos = Vec2::new(16.0, 25.0);
        assert_eq!(
            payload_guard(&guards, &conf),
            Some(1),
            "choose walking distance, not lower slot id or distance through a wall"
        );
        guards.fleet_me[2].pos = Vec2::new(16.0, 18.4);
        assert_eq!(
            payload_guard(&guards, &conf),
            Some(2),
            "retain a fighter already in capture"
        );
        guards.fleet_me[1].pos = Vec2::new(16.0, 17.5);
        assert_eq!(payload_guard(&guards, &conf), Some(1));
        guards.fleet_me[2].pos = Vec2::new(16.0, 17.1);
        assert_eq!(
            payload_guard(&guards, &conf),
            Some(1),
            "do not swap occupied guards just because another inches closer"
        );
        guards.fleet_me.remove(1);
        assert_eq!(
            payload_guard(&guards, &conf),
            Some(2),
            "replace a dead guard immediately"
        );
        let recycled = guards.fleet_me.add();
        assert_eq!(recycled, 1);
        guards.fleet_me[recycled].pos = Vec2::new(1.0, 31.0);
        assert_eq!(
            payload_guard(&guards, &conf),
            Some(2),
            "a respawned low slot must not steal capture duty from the front line"
        );
        guards.fleet_me[2].special = SpecialState::new(BotClass::Healer);
        guards.fleet_me[0].special = SpecialState::new(BotClass::Extractor);
        assert_eq!(
            payload_guard(&guards, &conf),
            Some(1),
            "only assign fighters"
        );

        let mut adaptive = GameState::new(&conf);
        let miner = adaptive.fleet_me.add();
        adaptive.fleet_me[miner].special = SpecialState::new(BotClass::Extractor);
        assert_eq!(
            battle_strategy(&adaptive, &conf).fabricator_next,
            BotClass::Battle,
            "field a fighter while the opening is unknown"
        );
        adaptive.tick = 20;
        assert_eq!(
            battle_strategy(&adaptive, &conf).fabricator_next,
            BotClass::Extractor,
            "default to the balanced economy after observing the opening"
        );
        for _ in 0..18 {
            adaptive.fleet_me.add();
        }
        for _ in 0..2 {
            let id = adaptive.fleet_me.add();
            adaptive.fleet_me[id].special = SpecialState::new(BotClass::Healer);
        }
        for _ in 0..9 {
            adaptive.fleet_other.add();
        }
        adaptive.fleet_other[0].special = SpecialState::new(BotClass::Extractor);
        assert_eq!(
            battle_strategy(&adaptive, &conf).fabricator_next,
            BotClass::Battle,
            "against a low-economy rush, prioritize fighters over more support"
        );
        for id in 5..9 {
            adaptive.fleet_other[id].special = SpecialState::new(BotClass::Healer);
        }
        let seen = observe_opponent(&adaptive, (0, 0));
        assert_eq!(seen, (1, 4));
        assert_eq!(
            battle_strategy_for_opponent(&adaptive, &conf, seen).fabricator_next,
            BotClass::Extractor,
            "retain mining income against sustained healing armies"
        );
        let second_miner = adaptive.fleet_me.add();
        adaptive.fleet_me[second_miner].special = SpecialState::new(BotClass::Extractor);
        assert_eq!(
            battle_strategy_for_opponent(&adaptive, &conf, seen).fabricator_next,
            BotClass::Extractor
        );
        let third_miner = adaptive.fleet_me.add();
        adaptive.fleet_me[third_miner].special = SpecialState::new(BotClass::Extractor);
        assert_eq!(
            battle_strategy_for_opponent(&adaptive, &conf, seen).fabricator_next,
            BotClass::Healer,
            "build sustained healing support against a healing formation"
        );
        for id in 5..9 {
            adaptive.fleet_other.remove(id);
        }
        assert_eq!(
            observe_opponent(&adaptive, seen),
            seen,
            "casualties must not erase the observed army composition"
        );
        assert_eq!(
            battle_strategy_for_opponent(&adaptive, &conf, seen).fabricator_next,
            BotClass::Healer
        );
        let third_healer = adaptive.fleet_me.add();
        adaptive.fleet_me[third_healer].special = SpecialState::new(BotClass::Healer);
        assert_eq!(
            battle_strategy_for_opponent(&adaptive, &conf, seen).fabricator_next,
            BotClass::Healer
        );
        assert_eq!(
            battle_strategy(&adaptive, &conf).fabricator_next,
            BotClass::Battle,
            "a fresh match must not inherit a previous opponent profile"
        );
        assert_eq!(observe_opponent(&GameState::new(&conf), (0, 0)), (0, 0));

        let mut economy = GameState::new(&conf);
        economy.tick = 20;
        for _ in 0..3 {
            assert_eq!(
                battle_strategy(&economy, &conf).fabricator_next,
                BotClass::Extractor
            );
            let id = economy.fleet_me.add();
            economy.fleet_me[id].special = SpecialState::new(BotClass::Extractor);
        }
        assert_eq!(
            battle_strategy(&economy, &conf).fabricator_next,
            BotClass::Battle
        );

        assert_eq!(battle_strategy_for_opponent(&economy, &conf, (8, 2)).fabricator_next,
            BotClass::Battle, "keep the funded opening unchanged against eight miners");
        economy.tick = 21;
        assert_eq!(battle_strategy_for_opponent(&economy, &conf, (7, 2)).fabricator_next,
            BotClass::Battle, "seven enemy miners must retain the existing three-miner economy");
        for _ in 0..2 {
            assert_eq!(battle_strategy_for_opponent(&economy, &conf, (8, 2)).fabricator_next,
                BotClass::Extractor, "eight enemy miners justify two extra economy slots");
            let id = economy.fleet_me.add();
            economy.fleet_me[id].special = SpecialState::new(BotClass::Extractor);
        }
        assert_eq!(battle_strategy_for_opponent(&economy, &conf, (8, 2)).fabricator_next,
            BotClass::Battle, "stop expanding at five miners");

        let mut crowd = GameState::new(&conf);
        for _ in 0..2 {
            let id = crowd.fleet_me.add();
            crowd.fleet_me[id].pos = Vec2::new(10.0, 10.0);
        }
        let left = spaced_move(
            &crowd,
            &conf,
            &crowd.fleet_me[0],
            Vec2::new(10.0, 10.0),
            false,
        );
        let right = spaced_move(
            &crowd,
            &conf,
            &crowd.fleet_me[1],
            Vec2::new(10.0, 10.0),
            false,
        );
        assert!(
            left.direction.dot(right.direction) < -0.9,
            "exactly overlapping allies must separate in opposite directions"
        );
        // Feed movement back for one second: a persistent clump must open up.
        for _ in 0..60 {
            let moves: Vec<_> = crowd
                .fleet_me
                .iter()
                .map(|bot| spaced_move(&crowd, &conf, bot, Vec2::new(10.0, 10.0), false))
                .collect();
            for bot in crowd.fleet_me.iter_mut() {
                bot.pos += moves[bot.id as usize].direction * conf.bot.speed;
            }
        }
        assert!(
            crowd.fleet_me[0].pos.dist(&crowd.fleet_me[1].pos)
                > conf.bot.radius + conf.bot.base_blaster_splash_radius,
            "shared navigation goals must not pull allies back into a splash-sized clump"
        );

        crowd.fleet_me[0].pos = Vec2::new(conf.bot.radius, 10.0);
        crowd.fleet_me[1].pos = Vec2::new(conf.bot.radius + 0.1, 10.0);
        let edge = spaced_move(
            &crowd,
            &conf,
            &crowd.fleet_me[0],
            Vec2::new(0.25, 12.0),
            false,
        );
        assert!(
            point_free(
                &conf,
                crowd.fleet_me[0].pos + edge.direction * conf.bot.speed
            ),
            "repulsion must not push a bot into the boundary"
        );
        conf.map[9][10] = MapTile::Wall;
        crowd.fleet_me[0].pos = Vec2::new(10.251, 10.5);
        crowd.fleet_me[1].pos = Vec2::new(10.35, 10.5);
        let wall = spaced_move(
            &crowd,
            &conf,
            &crowd.fleet_me[0],
            crowd.fleet_me[0].pos,
            false,
        );
        assert!(
            point_free(
                &conf,
                crowd.fleet_me[0].pos + wall.direction * conf.bot.speed
            ),
            "repulsion must not push a bot into a wall"
        );
        conf.map[9][10] = MapTile::Empty;

        let payload = crowd.payload_pos();
        crowd.fleet_me[0].pos = payload + Vec2::new(conf.payload.capture_radius - 0.01, 0.0);
        crowd.fleet_me[1].pos = crowd.fleet_me[0].pos - Vec2::new(0.1, 0.0);
        let guard = spaced_move(&crowd, &conf, &crowd.fleet_me[0], payload, true);
        assert!(
            (crowd.fleet_me[0].pos + guard.direction * conf.bot.speed).dist(&payload)
                <= conf.payload.capture_radius,
            "spacing must not abandon capture"
        );
        crowd.fleet_me[0].pos =
            payload + Vec2::new(conf.payload.radius + conf.bot.radius + 0.001, 0.0);
        crowd.fleet_me[1].pos = crowd.fleet_me[0].pos + Vec2::new(0.1, 0.0);
        let solid = spaced_move(
            &crowd,
            &conf,
            &crowd.fleet_me[0],
            crowd.fleet_me[0].pos,
            false,
        );
        assert!(
            (crowd.fleet_me[0].pos + solid.direction * conf.bot.speed).dist(&payload)
                >= conf.payload.radius + conf.bot.radius,
            "spacing must respect solid payload"
        );
        for _ in 0..5 {
            economy.fleet_me.add();
        }
        assert_eq!(
            battle_strategy(&economy, &conf).fabricator_next,
            BotClass::Healer
        );
        let id = economy.fleet_me.add();
        economy.fleet_me[id].special = SpecialState::new(BotClass::Healer);
        assert_eq!(
            battle_strategy(&economy, &conf).fabricator_next,
            BotClass::Battle
        );

        // A raided economy must still produce defenders instead of an endless miner queue.
        let mut raided = GameState::new(&conf);
        raided.tick = 1000;
        raided.fleet_other.add();
        raided.fleet_other[0].pos = raided.deposit_me.pos + Vec2::new(2.0, 0.0);
        assert_eq!(
            battle_strategy(&raided, &conf).fabricator_next,
            BotClass::Battle
        );
        for _ in 0..4 {
            raided.fleet_me.add();
        }
        assert_eq!(
            battle_strategy(&raided, &conf).fabricator_next,
            BotClass::Extractor,
            "resume economy after fielding four defenders"
        );
        raided.fleet_me = GameState::new(&conf).fleet_me;
        raided.fleet_other = GameState::new(&conf).fleet_other;
        assert_eq!(
            battle_strategy(&raided, &conf).fabricator_next,
            BotClass::Extractor,
            "unthreatened miner losses still get replaced"
        );

        let mut defense = GameState::new(&conf);
        defense.tick = 1000;
        defense.fleet_me.add();
        defense.fleet_me[0].pos = Vec2::new(27.0, 28.0);
        defense.fleet_other.add();
        defense.fleet_other[0].pos = Vec2::new(29.0, 28.0);
        defense.fleet_other[0].health = conf.bot.health;
        assert!(
            defense.fleet_other[0].pos.dist(&defense.payload_pos())
                > conf.bot.blaster_range + conf.payload.capture_radius
        );
        assert!(
            fires(&battle_strategy(&defense, &conf)),
            "fight nearby attackers even when they are far from the payload"
        );
        defense.fleet_other.add();
        defense.fleet_other[1].pos = Vec2::new(28.0, 30.0);
        defense.fleet_other[1].health = 1.0;
        let action = battle_strategy(&defense, &conf);
        assert!(
            matches!(action.bots[0].turn_action, TurnAction::TargetPosition { pos }
            if pos == defense.fleet_other[0].pos),
            "keep nearest-target pressure against light armies"
        );
        defense.fleet_me[0].angle =
            (defense.fleet_other[1].pos - defense.fleet_me[0].pos).angle_deg();
        let action = battle_strategy_for_opponent(&defense, &conf, (8, 0));
        assert!(
            matches!(action.bots[0].turn_action, TurnAction::TargetPosition { pos }
            if pos == defense.fleet_other[1].pos),
            "finish the exposed wounded opponent"
        );
        defense.fleet_other[1].invulnerable_until_tick = 1001;
        let action = battle_strategy_for_opponent(&defense, &conf, (8, 0));
        assert!(
            matches!(action.bots[0].turn_action, TurnAction::TargetPosition { pos }
            if pos == defense.fleet_other[0].pos),
            "do not prioritize an invulnerable wounded target"
        );
        defense.fleet_other[1].invulnerable_until_tick = 0;
        for _ in 0..4 {
            let id = defense.fleet_other.add();
            defense.fleet_other[id].special = SpecialState::new(BotClass::Healer);
            defense.fleet_other[id].pos = Vec2::new(1.0, 1.0);
            defense.fleet_other[id].health = conf.bot.health;
        }
        let action = battle_strategy(&defense, &conf);
        assert!(
            matches!(action.bots[0].turn_action, TurnAction::TargetPosition { pos }
            if pos == defense.fleet_other[0].pos),
            "healer-majority swarms retain nearest-target pressure"
        );
        let focused = battle_strategy_with_contest(&defense, &conf, (3, 4), true);
        assert!(matches!(focused.bots[0].turn_action, TurnAction::TargetPosition { pos }
            if pos == defense.fleet_other[1].pos), "finish an exposed patient in a learned supported contest");

        let mut opportunity = GameState::new(&conf);
        opportunity.tick = 1000;
        opportunity.fleet_me.add();
        opportunity.fleet_me[0].pos = Vec2::new(10.0, 16.0);
        for pos in [Vec2::new(11.0, 14.0), Vec2::new(14.0, 16.4)] {
            let id = opportunity.fleet_other.add();
            opportunity.fleet_other[id].pos = pos;
            opportunity.fleet_other[id].health = conf.bot.health;
        }
        let action = battle_strategy(&opportunity, &conf);
        assert!(
            fires(&action),
            "take an available shot when the nearest target needs more turning"
        );
        assert!(
            matches!(action.bots[0].turn_action, TurnAction::TargetPosition { pos }
            if pos == opportunity.fleet_other[1].pos),
            "aim at the target reachable by this tick's rotation"
        );
        opportunity.fleet_me[0].special = SpecialState::Battle {
            next_fire_tick: 1001,
            shot: StateOption::None,
        };
        assert!(
            !fires(&battle_strategy(&opportunity, &conf)),
            "opportunistic shots still respect cooldown"
        );
        let from = Vec2::new(4.0, 4.0);
        let target = Vec2::new(13.0, 4.0);
        assert_eq!(
            firing_position(&opportunity, &conf, from, target, false, true),
            from,
            "hold a clear long-range firing position against light armies"
        );
        assert_ne!(
            firing_position(&opportunity, &conf, from, target, false, false),
            from,
            "keep closer pressure against heavy economies"
        );

        let mut retreat = GameState::new(&conf);
        retreat.tick = 1000;
        retreat.fleet_me.add();
        retreat.fleet_me[0].pos = Vec2::new(10.0, 10.0);
        retreat.fleet_me[0].health = conf.bot.health;
        let medic = retreat.fleet_me.add();
        retreat.fleet_me[medic].pos = Vec2::new(7.0, 10.0);
        retreat.fleet_me[medic].special = SpecialState::new(BotClass::Healer);
        for pos in [Vec2::new(14.0, 10.0), Vec2::new(14.0, 12.0)] {
            let id = retreat.fleet_other.add();
            retreat.fleet_other[id].pos = pos;
            retreat.fleet_other[id].health = conf.bot.health;
        }
        let rally = regroup_position(&retreat, &conf, &retreat.fleet_me[0]).unwrap();
        assert!(
            rally.x < retreat.fleet_me[0].pos.x,
            "outnumbered fighters fall back"
        );
        assert!(
            rally.dist(&retreat.fleet_me[medic].pos) < conf.bot.base_heal_range,
            "regroup inside the supporting healer's range"
        );
        let action = battle_strategy(&retreat, &conf);
        assert!(
            action.bots[0].move_action.direction.x < 0.0,
            "retreat overrides advancing to the payload"
        );
        assert!(
            fires(&action),
            "continue taking legal shots while retreating"
        );
        for id in [0, 1] {
            retreat.fleet_other[id].health = 1.5;
        }
        assert!(
            regroup_position(&retreat, &conf, &retreat.fleet_me[0]).is_none(),
            "healthy fighters keep a winning fight"
        );
        retreat.fleet_me[0].health = 4.0;
        assert!(
            regroup_position(&retreat, &conf, &retreat.fleet_me[0]).is_some(),
            "wounded fighters regroup even before being outnumbered"
        );
        retreat.fleet_me[medic].pos = Vec2::new(16.0, 10.0);
        assert!(
            regroup_position(&retreat, &conf, &retreat.fleet_me[0]).is_none(),
            "do not retreat to a healer closer to the enemy group"
        );
        retreat.fleet_me[medic].special = SpecialState::new(BotClass::Extractor);
        assert!(
            regroup_position(&retreat, &conf, &retreat.fleet_me[0]).is_none(),
            "miners are not healing support"
        );

        // Healing formations hold healthy fighters forward, but recover wounded fighters.
        retreat.fleet_me[medic].special = SpecialState::new(BotClass::Healer);
        retreat.fleet_me[medic].pos = Vec2::new(7.0, 10.0);
        retreat.fleet_me[0].health = conf.bot.health;
        for id in [0, 1] {
            retreat.fleet_other[id].health = conf.bot.health;
        }
        for _ in 0..4 {
            let id = retreat.fleet_other.add();
            retreat.fleet_other[id].special = SpecialState::new(BotClass::Healer);
            retreat.fleet_other[id].pos = Vec2::new(1.0, 1.0);
        }
        let orders = battle_strategy(&retreat, &conf);
        assert!(
            orders.bots[0].move_action.direction.x >= 0.0,
            "healthy fighters must not abandon a healing-supported front line"
        );
        retreat.fleet_me[0].health = 4.0;
        let orders = battle_strategy(&retreat, &conf);
        assert!(
            orders.bots[0].move_action.direction.x < 0.0,
            "wounded fighters return to medics in healing formations"
        );
        assert!(fires(&orders), "healing formation retreats keep firing");
        retreat.fleet_me[medic].pos = Vec2::new(13.0, 10.0);
        assert!(
            battle_strategy(&retreat, &conf).bots[medic as usize]
                .move_action
                .direction
                .x
                < 0.0,
            "exposed medics move behind their patient"
        );
        // A lone medic must not park a moderately wounded army through the endgame.
        let mut depleted = GameState::new(&conf);
        for pos in [
            Vec2::new(10.0, 10.0),
            Vec2::new(10.0, 12.0),
            Vec2::new(8.0, 12.0),
            Vec2::new(6.0, 12.0),
        ] {
            let id = depleted.fleet_me.add();
            depleted.fleet_me[id].pos = pos;
            depleted.fleet_me[id].health = conf.bot.health;
        }
        let lone_medic = depleted.fleet_me.add();
        depleted.fleet_me[lone_medic].special = SpecialState::new(BotClass::Healer);
        depleted.fleet_me[lone_medic].pos = Vec2::new(7.0, 10.0);
        depleted.fleet_me[lone_medic].health = conf.bot.health;
        depleted.fleet_me[0].health = conf.bot.health * 0.55;
        depleted.tick = conf.max_ticks - conf.endgame_ticks - 1;
        assert!(
            battle_strategy_for_opponent(&depleted, &conf, (8, 10)).bots[0]
                .move_action
                .direction
                .x
                < 0.0,
            "before the endgame, wounded fighters still return to their medic"
        );
        depleted.tick += 1;
        assert!(
            battle_strategy_for_opponent(&depleted, &conf, (8, 10)).bots[0]
                .move_action
                .direction
                .x
                > 0.0,
            "a depleted endgame army returns moderately wounded fighters to the objective"
        );
        depleted.fleet_me[0].health = conf.bot.health * 0.3;
        assert!(
            battle_strategy_for_opponent(&depleted, &conf, (8, 10)).bots[0]
                .move_action
                .direction
                .x
                < 0.0,
            "critically wounded fighters still seek healing"
        );
        depleted.fleet_me[0].health = conf.bot.health * 0.55;
        let second_medic = depleted.fleet_me.add();
        depleted.fleet_me[second_medic].special = SpecialState::new(BotClass::Healer);
        depleted.fleet_me[second_medic].pos = Vec2::new(7.0, 12.0);
        assert!(
            battle_strategy_for_opponent(&depleted, &conf, (8, 10)).bots[0]
                .move_action
                .direction
                .x
                < 0.0,
            "adequately supported endgame armies retain normal healing retreats"
        );

        let mut rear = GameState::new(&conf);
        rear.fleet_me.add();
        rear.fleet_me[0].pos = Vec2::new(10.0, 10.0);
        rear.fleet_me[0].health = 4.0;
        let medic = rear.fleet_me.add();
        rear.fleet_me[medic].special = SpecialState::new(BotClass::Healer);
        rear.fleet_me[medic].pos = Vec2::new(8.0, 11.0);
        rear.fleet_me[medic].health = conf.bot.health;
        rear.fleet_other.add();
        rear.fleet_other[0].pos = Vec2::new(30.0, 10.0);
        let follow = spaced_move(
            &rear,
            &conf,
            &rear.fleet_me[medic],
            rear.fleet_me[0].pos,
            false,
        );
        let orders = battle_strategy_for_opponent(&rear, &conf, (8, 10));
        assert!(
            orders.bots[medic as usize]
                .move_action
                .direction
                .dist(&follow.direction)
                < EPSILON,
            "against heavy economies, distant fighters do not push medics behind their patients"
        );
        assert!(
            battle_strategy_for_opponent(&rear, &conf, (3, 10)).bots[medic as usize]
                .move_action
                .direction
                .dist(&follow.direction)
                > 0.1,
            "retain existing light-army medic positioning"
        );
        rear.fleet_other[0].pos = Vec2::new(14.0, 10.0);
        assert!(
            battle_strategy_for_opponent(&rear, &conf, (8, 10)).bots[medic as usize]
                .move_action
                .direction
                .dist(&follow.direction)
                > 0.1,
            "nearby fighters still make medics move behind their patients"
        );

        let mut turtle = GameState::new(&conf);
        turtle.tick = 1500;
        turtle.deposit_other.pos = Vec2::new(25.0, 10.0);
        turtle.fleet_me.add();
        turtle.fleet_me[0].pos = Vec2::new(18.0, 10.0);
        let guard = turtle.fleet_me.add();
        turtle.fleet_me[guard].pos = Vec2::new(15.0, 16.0);
        turtle.fleet_other.add();
        turtle.fleet_other[0].pos = Vec2::new(22.0, 10.0);
        assert!(!clear_shot(
            &turtle,
            &conf,
            turtle.fleet_me[0].pos,
            turtle.fleet_other[0].pos
        ));
        assert_eq!(payload_guard(&turtle, &conf), Some(guard));
        let expected = spaced_move(
            &turtle,
            &conf,
            &turtle.fleet_me[0],
            turtle.payload_pos(),
            false,
        );
        let orders = battle_strategy_for_opponent(&turtle, &conf, (8, 0));
        assert!(orders.bots[0].move_action.direction.dist(&expected.direction) < EPSILON,
            "against a defended economy, advance on the objective instead of detouring to a covered target");
        turtle.tick = 1499;
        assert!(
            battle_strategy_for_opponent(&turtle, &conf, (8, 0)).bots[0]
                .move_action
                .direction
                .dist(&expected.direction)
                > 0.1,
            "keep opening combat before changing objective pressure"
        );
        turtle.tick = 1500;
        assert!(
            battle_strategy_for_opponent(&turtle, &conf, (5, 0)).bots[0]
                .move_action
                .direction
                .dist(&expected.direction)
                > 0.1,
            "keep normal flanking against light economies"
        );
        // Replay 982: five early miners behind cover pull fighters away from support.
        turtle.tick = 500;
        turtle.fleet_other[0].special = SpecialState::new(BotClass::Extractor);
        let orders = battle_strategy_for_opponent(&turtle, &conf, (5, 4));
        assert!(orders.bots[0].move_action.direction.dist(&expected.direction) < EPSILON,
            "keep the payload route when an early supported army offers a blocked miner");
        turtle.fleet_other[0].special = SpecialState::new(BotClass::Battle);
        assert!(battle_strategy_for_opponent(&turtle, &conf, (5, 4)).bots[0]
            .move_action.direction.dist(&expected.direction) > 0.1,
            "the early miner exception must preserve combat-target flanking");
        turtle.tick = 1500;
        turtle.deposit_other.pos = Vec2::new(31.0, 31.0);
        assert!(
            battle_strategy_for_opponent(&turtle, &conf, (8, 0)).bots[0]
                .move_action
                .direction
                .dist(&expected.direction)
                > 0.1,
            "keep normal flanking when the enemy has left its home defense"
        );
        turtle.deposit_other.pos = Vec2::new(25.0, 10.0);
        turtle.fleet_other[0].pos = Vec2::new(18.0, 5.0);
        assert!(clear_shot(
            &turtle,
            &conf,
            turtle.fleet_me[0].pos,
            turtle.fleet_other[0].pos
        ));
        assert!(
            battle_strategy_for_opponent(&turtle, &conf, (8, 0)).bots[0]
                .move_action
                .direction
                .dist(&expected.direction)
                > 0.1,
            "an exposed target still uses the existing firing-position policy"
        );

        let mut rescue = GameState::new(&conf);
        rescue.tick = 1200;
        let medic = rescue.fleet_me.add();
        rescue.fleet_me[medic].special = SpecialState::new(BotClass::Healer);
        rescue.fleet_me[medic].pos = Vec2::new(8.0, 8.0);
        let guard = rescue.fleet_me.add();
        rescue.fleet_me[guard].pos = Vec2::new(10.0, 8.0);
        let wounded = rescue.fleet_me.add();
        rescue.fleet_me[wounded].pos = Vec2::new(5.0, 8.0);
        rescue.fleet_me[wounded].health = 4.0;
        for bot in rescue.fleet_me.iter_mut() { bot.health = conf.bot.health; }
        rescue.fleet_me[wounded].health = 4.0;
        let heavy = battle_strategy_for_opponent(&rescue, &conf, (8, 4));
        assert!(heavy.bots[medic as usize].move_action.direction.x < -0.5,
            "idle heavy-economy medic approaches a wounded fighter just outside heal range");
        let light = battle_strategy_for_opponent(&rescue, &conf, (3, 4));
        assert!(light.bots[medic as usize].move_action.direction.x > 0.5,
            "preserve light-economy escort selection");
        rescue.fleet_me[wounded].health = 7.0;
        assert!(battle_strategy_for_opponent(&rescue, &conf, (8, 4)).bots[medic as usize]
            .move_action.direction.x > 0.5,
            "minor damage must not divert idle medics from objective support");
        let mut opening = GameState::new(&conf);
        opening.tick = 20;
        for class in [
            BotClass::Extractor,
            BotClass::Extractor,
            BotClass::Extractor,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Healer,
            BotClass::Healer,
        ] {
            let id = opening.fleet_me.add();
            opening.fleet_me[id].special = SpecialState::new(class);
        }
        for class in [
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Battle,
            BotClass::Healer,
            BotClass::Healer,
        ] {
            let id = opening.fleet_other.add();
            opening.fleet_other[id].special = SpecialState::new(class);
        }
        assert_eq!(
            battle_strategy(&opening, &conf).fabricator_next,
            BotClass::Healer,
            "recognize an early two-healer opening before the enemy reaches four healers"
        );
    }
}
