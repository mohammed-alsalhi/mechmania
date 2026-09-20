// Generated from reviewed patches against frozen v12. Local sparring only.
use crate::core::Strategy;
pub fn get_strategy(name: &str) -> Option<Strategy> {
    match name {
        "grok-leash-healing-front" => Some(grok_leash_healing_front::get_strategy(0)),
        "grok-healer-hunter" => Some(grok_healer_hunter::get_strategy(0)),
        "grok-triple-hold-crush" => Some(grok_triple_hold_crush::get_strategy(0)),
        "grok-staged-army-wave" => Some(grok_staged_army_wave::get_strategy(0)),
        "grok-splash-clump-fire" => Some(grok_splash_clump_fire::get_strategy(0)),
        "grok-threshold-peel" => Some(grok_threshold_peel::get_strategy(0)),
        _ => None,
    }
}
mod grok_leash_healing_front {
    use crate::core::*;

    // Scout only this match; no network or opponent names are needed.
    pub fn get_strategy(_team: u8) -> Strategy {
        let seen = std::cell::Cell::new((0, 0));
        Box::new(move |state| {
            let observed = observe_opponent(state, seen.get());
            seen.set(observed);
            battle_strategy_for_opponent(state, get_config(), observed)
        })
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

    fn battle_strategy_for_opponent(
        state: &GameState,
        conf: &GameConfig,
        observed: (usize, usize),
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
        // Mining-heavy armies can scale behind just two early medics.
        let support = observed.1 >= 4
            || (observed.1 >= 2
                && (observed.0 >= 6 || (observed.0 <= 4 && enemy_fighters <= observed.1 * 3)));
        // Field fighters while the opening is still being revealed. Match early rush pressure,
        // then use the normal economy against balanced and mining-heavy armies.
        let miner_goal = if state.tick < 8 || rush { 1 } else { 3 };

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
        let healer_goal = if battles < 6 {
            0
        } else {
            (battles * 2 / 5).max(4).min(9)
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
        let mining_spot =
            state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

        let guard = payload_guard(state, conf);
        let mut covered = 0u32;
        let mut healing = [0u8; 32];
        let mut escorts = [0u8; 32];

        for bot_state in state.fleet_me.iter() {
            let bot_action = &mut action.bots[bot_state.id as usize];

            if bot_state.class() == BotClass::Extractor {
                bot_action.move_action = spaced_move(state, conf, bot_state, mining_spot, false);
                bot_action.turn_action = turn_towards(state.deposit_me.pos);
                bot_action.special_action = SpecialAction::Extractor { mine: true };
                continue;
            }

            if bot_state.class() == BotClass::Healer {
                bot_action.move_action = spaced_move(state, conf, bot_state, bot_state.pos, false);
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
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    });
                let follow = patient.or_else(|| {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Battle)
                        .min_by(|a, b| {
                            let cost = |ally: &BotState| {
                                ally.pos.dist(&bot_state.pos)
                                    + escorts[ally.id as usize] as f32 * conf.bot.base_heal_range
                            };
                            cost(a).total_cmp(&cost(b))
                        })
                });
                if let Some(ally) = follow {
                    escorts[ally.id as usize] += 1;
                    if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                        bot_action.move_action =
                            spaced_move(state, conf, bot_state, ally.pos, false);
                    }
                    {
                        if let Some(enemy) = state
                            .fleet_other
                            .iter()
                            .filter(|b| b.class() == BotClass::Battle)
                            .filter(|b| {
                                b.pos.dist(&ally.pos) < conf.bot.blaster_range + conf.bot.radius
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
                                    spaced_move(state, conf, bot_state, rear, false);
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
                    bot_action.move_action = spaced_move(state, conf, bot_state, payload, false);
                    bot_action.special_action = SpecialAction::Healer {
                        fire: false,
                        target: bot_state.id,
                    };
                }
                continue;
            }

            // Everyone advances when there is no nearby enemy; one fighter holds capture.
            let rally = {
                let wounded = bot_state.health <= conf.bot.health * 0.6;
                let threatened = state.fleet_other.iter().any(|enemy| {
                    enemy.class() == BotClass::Battle
                        && enemy.pos.dist(&bot_state.pos) <= conf.bot.blaster_range
                });
                if wounded || threatened {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Healer)
                        .filter_map(|ally| {
                            path_length(conf, bot_state.pos, ally.pos).map(|d| (ally, d))
                        })
                        .min_by(|(_, a), (_, b)| a.total_cmp(b))
                        .and_then(|(ally, distance)| {
                            if !wounded && distance <= 4.5 {
                                None
                            } else {
                                let meeting = ally.pos
                                    + (bot_state.pos - ally.pos).normalize_or_zero()
                                        * conf.bot.base_heal_range
                                        * 0.65;
                                if point_free(conf, meeting)
                                    && obstacles(state, conf)
                                        .iter()
                                        .all(|&(p, r)| meeting.dist(&p) > r + conf.bot.radius)
                                {
                                    Some(meeting)
                                } else {
                                    Some(ally.pos)
                                }
                            }
                        })
                        .or_else(|| regroup_position(state, conf, bot_state))
                } else {
                    None
                }
            };
            let contester = guard == Some(bot_state.id) && rally.is_none();
            bot_action.move_action =
                spaced_move(state, conf, bot_state, rally.unwrap_or(payload), contester);

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
                        if exposed && observed.0 >= 6 {
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
                if observed.0 >= 6
                    && state.tick >= 1500
                    && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range
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
                    false && support,
                )
            });
            bot_action.move_action = spaced_move(state, conf, bot_state, goal, contester);
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

    fn payload_guard(state: &GameState, conf: &GameConfig) -> Option<BotId> {
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
            .min_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)))
            .map(|(id, _)| id)
    }

    // Allies do not collide in the engine, so navigation alone piles them onto one point.
    // ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
    fn spaced_move(
        state: &GameState,
        conf: &GameConfig,
        bot: &BotState,
        goal: Vec2,
        guard: bool,
    ) -> MoveAction {
        let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
        travel.sanitize();
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
        let in_position =
            !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
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
            if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius)
            {
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
}
mod grok_healer_hunter {
    use crate::core::*;

    // Scout only this match; no network or opponent names are needed.
    pub fn get_strategy(_team: u8) -> Strategy {
        let seen = std::cell::Cell::new((0, 0));
        Box::new(move |state| {
            let observed = observe_opponent(state, seen.get());
            seen.set(observed);
            battle_strategy_for_opponent(state, get_config(), observed)
        })
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

    fn battle_strategy_for_opponent(
        state: &GameState,
        conf: &GameConfig,
        observed: (usize, usize),
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
        // Mining-heavy armies can scale behind just two early medics.
        let support = observed.1 >= 4
            || (observed.1 >= 2
                && (observed.0 >= 6 || (observed.0 <= 4 && enemy_fighters <= observed.1 * 3)));
        // Field fighters while the opening is still being revealed. Match early rush pressure,
        // then use the normal economy against balanced and mining-heavy armies.
        let miner_goal = if state.tick < 8 || rush { 1 } else { 3 };

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
        let healer_goal = if battles < 4 {
            0
        } else if battles < 8 {
            1
        } else {
            2
        };
        let _ = rush;
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
        let mining_spot =
            state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

        let guard = payload_guard(state, conf);
        let mut covered = 0u32;
        let mut healing = [0u8; 32];

        for bot_state in state.fleet_me.iter() {
            let bot_action = &mut action.bots[bot_state.id as usize];

            if bot_state.class() == BotClass::Extractor {
                bot_action.move_action = spaced_move(state, conf, bot_state, mining_spot, false);
                bot_action.turn_action = turn_towards(state.deposit_me.pos);
                bot_action.special_action = SpecialAction::Extractor { mine: true };
                continue;
            }

            if bot_state.class() == BotClass::Healer {
                bot_action.move_action = spaced_move(state, conf, bot_state, bot_state.pos, false);
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
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    });
                let follow = patient.or_else(|| {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Battle)
                        .min_by(|a, b| a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload)))
                });
                if let Some(ally) = follow {
                    if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                        bot_action.move_action =
                            spaced_move(state, conf, bot_state, ally.pos, false);
                    }
                    if support {
                        if let Some(enemy) = state
                            .fleet_other
                            .iter()
                            .filter(|b| b.class() == BotClass::Battle)
                            .filter(|b| {
                                observed.0 < 6
                                    || b.pos.dist(&ally.pos)
                                        < conf.bot.blaster_range + conf.bot.radius
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
                                    spaced_move(state, conf, bot_state, rear, false);
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
                    bot_action.move_action = spaced_move(state, conf, bot_state, payload, false);
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
                } else {
                    None
                }
            } else {
                regroup_position(state, conf, bot_state)
            };
            let contester = guard == Some(bot_state.id) && rally.is_none();
            bot_action.move_action =
                spaced_move(state, conf, bot_state, rally.unwrap_or(payload), contester);

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
                        u32::from(enemy.class() != BotClass::Healer),
                        (bot_state.pos.dist_sq(&enemy.pos) * 1000.0) as u32,
                    )
                });
            let enemy = match enemy {
                Some(enemy) => enemy,
                None => continue,
            };
            let goal = rally.unwrap_or_else(|| {
                // Do not detour into a defended economy when cover blocks our chosen shot.
                if observed.0 >= 6
                    && state.tick >= 1500
                    && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range
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
                    false && support,
                )
            });
            bot_action.move_action = spaced_move(state, conf, bot_state, goal, contester);
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
            if hits == 0 && bot_state.next_fire_tick() <= state.tick {
                for other in state.fleet_other.iter() {
                    let candidate = other.pos + other.vel;
                    let desired = (candidate - origin).angle_deg();
                    let angle = bot_state.angle
                        + diff_degrees(desired, bot_state.angle)
                            .clamp(-conf.bot.turn_speed, conf.bot.turn_speed);
                    let available = shot_targets(state, conf, origin, angle) & !covered;
                    let healer_hit = other.class() == BotClass::Healer && available != 0;
                    if available.count_ones() > hits.count_ones()
                        || (hits == 0 && available != 0)
                        || healer_hit
                    {
                        aim = candidate;
                        hits = available;
                        if healer_hit {
                            break;
                        }
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

    fn payload_guard(state: &GameState, conf: &GameConfig) -> Option<BotId> {
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
            .min_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)))
            .map(|(id, _)| id)
    }

    // Allies do not collide in the engine, so navigation alone piles them onto one point.
    // ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
    fn spaced_move(
        state: &GameState,
        conf: &GameConfig,
        bot: &BotState,
        goal: Vec2,
        guard: bool,
    ) -> MoveAction {
        let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
        travel.sanitize();
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
        let in_position =
            !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
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
            if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius)
            {
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
}
mod grok_triple_hold_crush {
    use crate::core::*;

    // Scout only this match; no network or opponent names are needed.
    pub fn get_strategy(_team: u8) -> Strategy {
        let seen = std::cell::Cell::new((0, 0));
        Box::new(move |state| {
            let observed = observe_opponent(state, seen.get());
            seen.set(observed);
            battle_strategy_for_opponent(state, get_config(), observed)
        })
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

    fn battle_strategy_for_opponent(
        state: &GameState,
        conf: &GameConfig,
        observed: (usize, usize),
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
        // Mining-heavy armies can scale behind just two early medics.
        let support = observed.1 >= 4
            || (observed.1 >= 2
                && (observed.0 >= 6 || (observed.0 <= 4 && enemy_fighters <= observed.1 * 3)));
        // Field fighters while the opening is still being revealed. Match early rush pressure,
        // then use the normal economy against balanced and mining-heavy armies.
        let miner_goal = if state.tick < 8 || rush { 1 } else { 3 };

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
        let healer_goal = if battles < 6 {
            0
        } else {
            (battles / 5).max(4).min(6)
        };
        let _ = rush;
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
        let mining_spot =
            state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

        let guards = payload_guards(state, conf, 3);
        let mut covered = 0u32;
        let mut healing = [0u8; 32];

        for bot_state in state.fleet_me.iter() {
            let bot_action = &mut action.bots[bot_state.id as usize];

            if bot_state.class() == BotClass::Extractor {
                bot_action.move_action = spaced_move(state, conf, bot_state, mining_spot, false);
                bot_action.turn_action = turn_towards(state.deposit_me.pos);
                bot_action.special_action = SpecialAction::Extractor { mine: true };
                continue;
            }

            if bot_state.class() == BotClass::Healer {
                bot_action.move_action = spaced_move(state, conf, bot_state, bot_state.pos, false);
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
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    });
                let follow = patient.or_else(|| {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Battle)
                        .min_by(|a, b| a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload)))
                });
                if let Some(ally) = follow {
                    if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                        bot_action.move_action =
                            spaced_move(state, conf, bot_state, ally.pos, false);
                    }
                    if support {
                        if let Some(enemy) = state
                            .fleet_other
                            .iter()
                            .filter(|b| b.class() == BotClass::Battle)
                            .filter(|b| {
                                observed.0 < 6
                                    || b.pos.dist(&ally.pos)
                                        < conf.bot.blaster_range + conf.bot.radius
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
                                    spaced_move(state, conf, bot_state, rear, false);
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
                    bot_action.move_action = spaced_move(state, conf, bot_state, payload, false);
                    bot_action.special_action = SpecialAction::Healer {
                        fire: false,
                        target: bot_state.id,
                    };
                }
                continue;
            }

            // Everyone advances when there is no nearby enemy; one fighter holds capture.
            let rally = if guards & (1 << bot_state.id) != 0 {
                None
            } else if support {
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
                } else {
                    None
                }
            } else {
                regroup_position(state, conf, bot_state)
            };
            let contester = guards & (1 << bot_state.id) != 0;
            bot_action.move_action =
                spaced_move(state, conf, bot_state, rally.unwrap_or(payload), contester);

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
                        if exposed && observed.0 >= 6 {
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
                if observed.0 >= 6
                    && state.tick >= 1500
                    && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range
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
            bot_action.move_action = spaced_move(state, conf, bot_state, goal, contester);
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

    fn payload_guards(state: &GameState, conf: &GameConfig, count: usize) -> u32 {
        let payload = state.payload_pos();
        let mut mask = 0u32;
        for _ in 0..count {
            let next = state
                .fleet_me
                .iter()
                .filter(|bot| bot.class() == BotClass::Battle)
                .filter(|bot| mask & (1 << bot.id) == 0)
                .filter_map(|bot| {
                    let distance = if bot.pos.dist(&payload) <= conf.payload.capture_radius {
                        Some(0.0)
                    } else {
                        path_length(conf, bot.pos, payload)
                    };
                    distance.map(|distance| (bot.id, distance))
                })
                .min_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)));
            match next {
                Some((id, _)) => mask |= 1 << id,
                None => break,
            }
        }
        mask
    }

    // Allies do not collide in the engine, so navigation alone piles them onto one point.
    // ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
    fn spaced_move(
        state: &GameState,
        conf: &GameConfig,
        bot: &BotState,
        goal: Vec2,
        guard: bool,
    ) -> MoveAction {
        let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
        travel.sanitize();
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
        let in_position =
            !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
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
            if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius)
            {
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
}
mod grok_staged_army_wave {
    use crate::core::*;

    // Scout only this match; no network or opponent names are needed.
    pub fn get_strategy(_team: u8) -> Strategy {
        let seen = std::cell::Cell::new((0, 0));
        Box::new(move |state| {
            let observed = observe_opponent(state, seen.get());
            seen.set(observed);
            battle_strategy_for_opponent(state, get_config(), observed)
        })
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

    fn battle_strategy_for_opponent(
        state: &GameState,
        conf: &GameConfig,
        observed: (usize, usize),
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
        // Mining-heavy armies can scale behind just two early medics.
        let support = observed.1 >= 4
            || (observed.1 >= 2
                && (observed.0 >= 6 || (observed.0 <= 4 && enemy_fighters <= observed.1 * 3)));
        // Field fighters while the opening is still being revealed. Match early rush pressure,
        // then use the normal economy against balanced and mining-heavy armies.
        let miner_goal = if state.tick < 8 || rush { 1 } else { 3 };

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
        let healer_goal = if battles < 8 {
            0
        } else {
            (battles / 3).max(4).min(8)
        };
        let _ = rush;
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
        let mining_spot =
            state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

        let hold = {
            let candidate = state.deposit_me.pos
                + (payload - state.deposit_me.pos).normalize_or_zero()
                    * (conf.deposit.radius + 8.0);
            if point_free(conf, candidate)
                && obstacles(state, conf)
                    .iter()
                    .all(|&(p, r)| candidate.dist(&p) > r + conf.bot.radius)
            {
                candidate
            } else {
                state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + 4.0)
            }
        };
        let staging = !state.in_endgame(conf)
            && !home_under_attack
            && (battles < 8 || healers < 4)
            && state.tick < 2200;
        let objective = if staging { hold } else { payload };
        let guard = payload_guard(state, conf);
        let mut covered = 0u32;
        let mut healing = [0u8; 32];

        for bot_state in state.fleet_me.iter() {
            let bot_action = &mut action.bots[bot_state.id as usize];

            if bot_state.class() == BotClass::Extractor {
                bot_action.move_action = spaced_move(state, conf, bot_state, mining_spot, false);
                bot_action.turn_action = turn_towards(state.deposit_me.pos);
                bot_action.special_action = SpecialAction::Extractor { mine: true };
                continue;
            }

            if bot_state.class() == BotClass::Healer {
                bot_action.move_action = spaced_move(state, conf, bot_state, bot_state.pos, false);
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
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    });
                let follow = patient.or_else(|| {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Battle)
                        .min_by(|a, b| a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload)))
                });
                if let Some(ally) = follow {
                    if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                        bot_action.move_action =
                            spaced_move(state, conf, bot_state, ally.pos, false);
                    }
                    if support {
                        if let Some(enemy) = state
                            .fleet_other
                            .iter()
                            .filter(|b| b.class() == BotClass::Battle)
                            .filter(|b| {
                                observed.0 < 6
                                    || b.pos.dist(&ally.pos)
                                        < conf.bot.blaster_range + conf.bot.radius
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
                                    spaced_move(state, conf, bot_state, rear, false);
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
                    bot_action.move_action = spaced_move(state, conf, bot_state, objective, false);
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
                } else {
                    None
                }
            } else {
                regroup_position(state, conf, bot_state)
            };
            let contester = guard == Some(bot_state.id) && rally.is_none();
            bot_action.move_action = spaced_move(
                state,
                conf,
                bot_state,
                rally.unwrap_or(objective),
                contester,
            );

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
                        if exposed && observed.0 >= 6 {
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
                if staging {
                    return objective;
                }
                // Do not detour into a defended economy when cover blocks our chosen shot.
                if observed.0 >= 6
                    && state.tick >= 1500
                    && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range
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
            bot_action.move_action = spaced_move(state, conf, bot_state, goal, contester);
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

    fn payload_guard(state: &GameState, conf: &GameConfig) -> Option<BotId> {
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
            .min_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)))
            .map(|(id, _)| id)
    }

    // Allies do not collide in the engine, so navigation alone piles them onto one point.
    // ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
    fn spaced_move(
        state: &GameState,
        conf: &GameConfig,
        bot: &BotState,
        goal: Vec2,
        guard: bool,
    ) -> MoveAction {
        let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
        travel.sanitize();
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
        let in_position =
            !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
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
            if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius)
            {
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
}
mod grok_splash_clump_fire {
    use crate::core::*;

    // Scout only this match; no network or opponent names are needed.
    pub fn get_strategy(_team: u8) -> Strategy {
        let seen = std::cell::Cell::new((0, 0));
        Box::new(move |state| {
            let observed = observe_opponent(state, seen.get());
            seen.set(observed);
            battle_strategy_for_opponent(state, get_config(), observed)
        })
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

    fn battle_strategy_for_opponent(
        state: &GameState,
        conf: &GameConfig,
        observed: (usize, usize),
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
        // Mining-heavy armies can scale behind just two early medics.
        let support = observed.1 >= 4
            || (observed.1 >= 2
                && (observed.0 >= 6 || (observed.0 <= 4 && enemy_fighters <= observed.1 * 3)));
        // Field fighters while the opening is still being revealed. Match early rush pressure,
        // then use the normal economy against balanced and mining-heavy armies.
        let miner_goal = if state.tick < 8 || rush { 1 } else { 3 };

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
        let healer_goal = if battles < 5 {
            0
        } else {
            (battles / 3).max(4).min(10)
        };
        let _ = rush;
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
        let mining_spot =
            state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

        let guard = payload_guard(state, conf);
        let mut covered = 0u32;
        let mut healing = [0u8; 32];

        for bot_state in state.fleet_me.iter() {
            let bot_action = &mut action.bots[bot_state.id as usize];

            if bot_state.class() == BotClass::Extractor {
                bot_action.move_action = spaced_move(state, conf, bot_state, mining_spot, false);
                bot_action.turn_action = turn_towards(state.deposit_me.pos);
                bot_action.special_action = SpecialAction::Extractor { mine: true };
                continue;
            }

            if bot_state.class() == BotClass::Healer {
                bot_action.move_action = spaced_move(state, conf, bot_state, bot_state.pos, false);
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
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    });
                let follow = patient.or_else(|| {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Battle)
                        .min_by(|a, b| a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload)))
                });
                if let Some(ally) = follow {
                    if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                        bot_action.move_action =
                            spaced_move(state, conf, bot_state, ally.pos, false);
                    }
                    if support {
                        if let Some(enemy) = state
                            .fleet_other
                            .iter()
                            .filter(|b| b.class() == BotClass::Battle)
                            .filter(|b| {
                                observed.0 < 6
                                    || b.pos.dist(&ally.pos)
                                        < conf.bot.blaster_range + conf.bot.radius
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
                                    spaced_move(state, conf, bot_state, rear, false);
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
                    bot_action.move_action = spaced_move(state, conf, bot_state, payload, false);
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
                } else {
                    None
                }
            } else {
                regroup_position(state, conf, bot_state)
            };
            let contester = guard == Some(bot_state.id) && rally.is_none();
            bot_action.move_action =
                spaced_move(state, conf, bot_state, rally.unwrap_or(payload), contester);

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
                        if exposed {
                            32 - state
                                .fleet_other
                                .iter()
                                .filter(|other| {
                                    other.pos.dist(&enemy.pos)
                                        <= conf.bot.base_blaster_splash_radius
                                            + conf.bot.radius * 2.0
                                        && other.invulnerable_until_tick <= state.tick
                                })
                                .count() as u32
                        } else {
                            32
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
                if observed.0 >= 6
                    && state.tick >= 1500
                    && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range
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
            bot_action.move_action = spaced_move(state, conf, bot_state, goal, contester);
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

    fn payload_guard(state: &GameState, conf: &GameConfig) -> Option<BotId> {
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
            .min_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)))
            .map(|(id, _)| id)
    }

    // Allies do not collide in the engine, so navigation alone piles them onto one point.
    // ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
    fn spaced_move(
        state: &GameState,
        conf: &GameConfig,
        bot: &BotState,
        goal: Vec2,
        guard: bool,
    ) -> MoveAction {
        let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
        travel.sanitize();
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
        let strength = 3.0;
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
        let in_position =
            !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
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
            if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius)
            {
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
}
mod grok_threshold_peel {
    use crate::core::*;

    // Scout only this match; no network or opponent names are needed.
    pub fn get_strategy(_team: u8) -> Strategy {
        let seen = std::cell::Cell::new((0, 0));
        Box::new(move |state| {
            let observed = observe_opponent(state, seen.get());
            seen.set(observed);
            battle_strategy_for_opponent(state, get_config(), observed)
        })
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

    fn battle_strategy_for_opponent(
        state: &GameState,
        conf: &GameConfig,
        observed: (usize, usize),
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
        // Mining-heavy armies can scale behind just two early medics.
        let support = observed.1 >= 4
            || (observed.1 >= 2
                && (observed.0 >= 6 || (observed.0 <= 4 && enemy_fighters <= observed.1 * 3)));
        // Field fighters while the opening is still being revealed. Match early rush pressure,
        // then use the normal economy against balanced and mining-heavy armies.
        let miner_goal = if state.tick < 8 || rush { 1 } else { 3 };

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
        let healer_goal = if battles < 6 {
            0
        } else {
            (battles / 4).max(4).min(8)
        };
        let _ = rush;
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
        let mining_spot =
            state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

        let guard = payload_guard(state, conf);
        let mut covered = 0u32;
        let mut healing = [0u8; 32];

        for bot_state in state.fleet_me.iter() {
            let bot_action = &mut action.bots[bot_state.id as usize];

            if bot_state.class() == BotClass::Extractor {
                bot_action.move_action = spaced_move(state, conf, bot_state, mining_spot, false);
                bot_action.turn_action = turn_towards(state.deposit_me.pos);
                bot_action.special_action = SpecialAction::Extractor { mine: true };
                continue;
            }

            if bot_state.class() == BotClass::Healer {
                bot_action.move_action = spaced_move(state, conf, bot_state, bot_state.pos, false);
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
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    });
                let follow = patient.or_else(|| {
                    state
                        .fleet_me
                        .iter()
                        .filter(|ally| ally.class() == BotClass::Battle)
                        .min_by(|a, b| a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload)))
                });
                if let Some(ally) = follow {
                    if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                        bot_action.move_action =
                            spaced_move(state, conf, bot_state, ally.pos, false);
                    }
                    if support {
                        if let Some(enemy) = state
                            .fleet_other
                            .iter()
                            .filter(|b| b.class() == BotClass::Battle)
                            .filter(|b| {
                                observed.0 < 6
                                    || b.pos.dist(&ally.pos)
                                        < conf.bot.blaster_range + conf.bot.radius
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
                                    spaced_move(state, conf, bot_state, rear, false);
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
                    bot_action.move_action = spaced_move(state, conf, bot_state, payload, false);
                    bot_action.special_action = SpecialAction::Healer {
                        fire: false,
                        target: bot_state.id,
                    };
                }
                continue;
            }

            // Everyone advances when there is no nearby enemy; one fighter holds capture.
            let rally = if bot_state.health <= conf.bot.blaster_damage {
                state
                    .fleet_me
                    .iter()
                    .filter(|ally| ally.class() == BotClass::Healer)
                    .filter_map(|ally| {
                        path_length(conf, bot_state.pos, ally.pos).map(|d| (ally, d))
                    })
                    .min_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(ally, _)| ally.pos)
                    .or_else(|| regroup_position(state, conf, bot_state))
            } else {
                None
            };
            let contester = guard == Some(bot_state.id) && rally.is_none();
            bot_action.move_action =
                spaced_move(state, conf, bot_state, rally.unwrap_or(payload), contester);

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
                    let on_point = enemy.pos.dist(&payload) <= conf.payload.capture_radius;
                    (
                        !exposed,
                        if on_point { 0u32 } else { 1u32 },
                        (enemy.health * 100.0) as u32,
                        (bot_state.pos.dist_sq(&enemy.pos) * 1000.0) as u32,
                    )
                });
            let enemy = match enemy {
                Some(enemy) => enemy,
                None => continue,
            };
            let goal = rally.unwrap_or_else(|| {
                // Do not detour into a defended economy when cover blocks our chosen shot.
                if observed.0 >= 6
                    && state.tick >= 1500
                    && enemy.pos.dist(&state.deposit_other.pos) < conf.bot.blaster_range
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
            bot_action.move_action = spaced_move(state, conf, bot_state, goal, contester);
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

    fn payload_guard(state: &GameState, conf: &GameConfig) -> Option<BotId> {
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
            .min_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)))
            .map(|(id, _)| id)
    }

    // Allies do not collide in the engine, so navigation alone piles them onto one point.
    // ponytail: local repulsion can still bunch in corridors; add formation routing only if replays require it.
    fn spaced_move(
        state: &GameState,
        conf: &GameConfig,
        bot: &BotState,
        goal: Vec2,
        guard: bool,
    ) -> MoveAction {
        let mut travel = move_bot(navigate_to(conf, bot.pos, goal));
        travel.sanitize();
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
        let in_position =
            !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
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
            if let Some(distance) = ray_circle(origin, dir, enemy.pos + enemy.vel, conf.bot.radius)
            {
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
}
