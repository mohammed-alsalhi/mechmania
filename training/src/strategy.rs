// Frozen v4 combat basis with explicit training-profile variations. Does not import the submitted strategy.
use crate::core::*;
use crate::Profile;

pub fn battle_strategy(state: &GameState, conf: &GameConfig, profile: &Profile) -> FleetAction {
    let mut action = FleetAction::new();

    let payload = state.payload_pos();

    let mut next_bot = BotClass::Battle;

    if state
        .fleet_me
        .iter()
        .filter(|bot| bot.class() == BotClass::Extractor)
        .count()
        < if profile.healing_ball && !profile.mobilize && state.tick < 8 {
            1
        } else if state.tick >= profile.switch_tick {
            profile.late_miners.unwrap_or(profile.miners)
        } else {
            profile.miners
        }
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
    let healer_goal = if profile.mobilize && state.tick < 20 {
        profile.opening_healers.unwrap_or(battles / 8)
    } else if profile.heal_every == 0.0 {
        profile.healers
    } else {
        ((battles as f32 / profile.heal_every) as usize).min(profile.healers)
    };
    if next_bot == BotClass::Battle && battles >= profile.fighter_floor && healers < healer_goal {
        next_bot = BotClass::Healer;
    }

    let mining_spot = state.deposit_me.pos + Vec2::new(0.0, conf.deposit.radius + conf.bot.radius);

    let staging = profile.staged_push && state.tick < profile.switch_tick;
    let guards = payload_guards(
        state,
        conf,
        if staging {
            profile.guards.min(2)
        } else {
            profile.guards
        },
    );
    let center = if state.tick < profile.delay_push {
        state.deposit_me.pos
    } else if profile.raid {
        state.deposit_other.pos
    } else {
        payload
    };
    // ponytail: fixed-map defensive posts from match 381; regenerate for a different map.
    let home_posts: Vec<_> = if profile.mobilize && (state.tick < profile.delay_push || staging) {
        (0..32)
            .map(|slot| {
                state.deposit_me.pos
                    + Vec2::new(
                        (slot % 8) as f32 * 1.05 - 1.5,
                        (slot / 8) as f32 * 1.05 - 2.5,
                    )
            })
            .filter(|&p| {
                point_free(conf, p)
                    && p.dist(&state.deposit_me.pos) > conf.deposit.radius + conf.bot.radius
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut covered = 0u32;
    let mut healing = [0u8; 32];
    let mut escorts = [0u8; 32];

    for bot_state in state.fleet_me.iter() {
        let bot_action = &mut action.bots[bot_state.id as usize];

        if bot_state.class() == BotClass::Extractor {
            // Free the miner slots for a paid combat wave before rush orders close.
            // The engine double-removes a miner shot dead on its retirement tick.
            // Retire only outside possible incoming shot/splash reach.
            bot_action.self_destruct = profile.mobilize
                && state.tick >= profile.switch_tick
                && !state.fleet_other.iter().any(|enemy| {
                    enemy.class() == BotClass::Battle
                        && enemy.pos.dist(&bot_state.pos)
                            <= conf.bot.blaster_range
                                + conf.bot.speed * 2.0
                                + conf.bot.base_blaster_splash_radius
                                + conf.bot.radius
                });
            // Replay 982 shows miners withdrawing while their fighters hold the center.
            // This escape vector is a stress approximation, not recovered opponent logic.
            let mine_goal = if profile.evade_miners {
                state.fleet_other.iter()
                    .filter(|enemy| enemy.class() == BotClass::Battle)
                    .filter(|enemy| enemy.pos.dist(&bot_state.pos) < conf.bot.blaster_range * 1.1)
                    .min_by(|a, b| a.pos.dist_sq(&bot_state.pos).total_cmp(&b.pos.dist_sq(&bot_state.pos)))
                    .map_or(mining_spot, |enemy| bot_state.pos
                        + (bot_state.pos - enemy.pos).normalize_or_zero() * 2.0)
            } else { mining_spot };
            bot_action.move_action =
                spaced_move(state, conf, bot_state, mine_goal, false, profile.spacing);
            bot_action.turn_action = turn_towards(state.deposit_me.pos);
            bot_action.special_action = SpecialAction::Extractor { mine: true };
            continue;
        }

        if bot_state.class() == BotClass::Healer {
            bot_action.move_action = spaced_move(
                state,
                conf,
                bot_state,
                bot_state.pos,
                false,
                profile.spacing,
            );
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
                .min_by(|a, b| {
                    if profile.coordinated {
                        let ready = |ally: &BotState| {
                            diff_degrees((ally.pos - bot_state.pos).angle_deg(), bot_state.angle)
                                .abs()
                                <= conf.bot.base_heal_arc_deg * 0.5 + conf.bot.turn_speed
                        };
                        (a.health > conf.bot.blaster_damage)
                            .cmp(&(b.health > conf.bot.blaster_damage))
                            .then(ready(b).cmp(&ready(a)))
                            .then(a.health.total_cmp(&b.health))
                    } else {
                        a.health.total_cmp(&b.health)
                    }
                });
            let follow = patient.or_else(|| {
                state
                    .fleet_me
                    .iter()
                    .filter(|ally| ally.class() == BotClass::Battle)
                    .min_by(|a, b| {
                        if profile.coordinated {
                            let cost = |ally: &BotState| {
                                ally.pos.dist(&bot_state.pos)
                                    + escorts[ally.id as usize] as f32 * conf.bot.base_heal_range
                            };
                            cost(a).total_cmp(&cost(b))
                        } else if profile.mobilize && (state.tick < profile.delay_push || staging) {
                            (a.health >= conf.bot.health)
                                .cmp(&(b.health >= conf.bot.health))
                                .then(
                                    a.pos
                                        .dist_sq(&bot_state.pos)
                                        .total_cmp(&b.pos.dist_sq(&bot_state.pos)),
                                )
                        } else {
                            a.pos.dist_sq(&payload).total_cmp(&b.pos.dist_sq(&payload))
                        }
                    })
            });
            if let Some(ally) = follow {
                escorts[ally.id as usize] += 1;
                if bot_state.pos.dist(&ally.pos) > conf.bot.base_heal_range * 0.65 {
                    bot_action.move_action =
                        spaced_move(state, conf, bot_state, ally.pos, false, profile.spacing);
                }
                if profile.healing_ball {
                    if let Some(enemy) = state
                        .fleet_other
                        .iter()
                        .filter(|b| b.class() == BotClass::Battle)
                        .filter(|b| {
                            !profile.coordinated
                                || b.pos.dist(&ally.pos) <= conf.bot.blaster_range + conf.bot.radius
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
                                spaced_move(state, conf, bot_state, rear, false, profile.spacing);
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
                bot_action.move_action =
                    spaced_move(state, conf, bot_state, center, false, profile.spacing);
                bot_action.special_action = SpecialAction::Healer {
                    fire: false,
                    target: bot_state.id,
                };
            }
            continue;
        }

        let finishing = profile.finish_push && state.capture >= 0.8;
        let recovery = if !finishing
            && profile.healing_ball
            && bot_state.health <= conf.bot.health * 0.6
        {
            state
                .fleet_me
                .iter()
                .filter(|ally| ally.class() == BotClass::Healer)
                .filter_map(|ally| path_length(conf, bot_state.pos, ally.pos).map(|d| (ally, d)))
                .min_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(ally, _)| ally.pos)
        } else {
            None
        };
        // Keep reinforcements within support instead of feeding isolated fighters forward.
        let recovery = recovery.or_else(|| {
            if finishing
                || profile.medic_leash == 0.0
                || !state.fleet_other.iter().any(|enemy| {
                    enemy.class() == BotClass::Battle
                        && enemy.pos.dist(&bot_state.pos) <= conf.bot.blaster_range
                })
            {
                return None;
            }
            let medic = state
                .fleet_me
                .iter()
                .filter(|ally| ally.class() == BotClass::Healer)
                .filter_map(|ally| path_length(conf, bot_state.pos, ally.pos).map(|d| (ally, d)))
                .min_by(|(_, a), (_, b)| a.total_cmp(b))?;
            if medic.1 <= profile.medic_leash {
                return None;
            }
            let meeting = medic.0.pos
                + (bot_state.pos - medic.0.pos).normalize_or_zero()
                    * conf.bot.base_heal_range
                    * 0.65;
            if point_free(conf, meeting)
                && obstacles(state, conf)
                    .iter()
                    .all(|&(p, r)| meeting.dist(&p) > r + conf.bot.radius)
            {
                Some(meeting)
            } else {
                Some(medic.0.pos)
            }
        });
        let advancing =
            state.tick >= profile.delay_push && (!staging || guards & (1 << bot_state.id) != 0);
        let home = if home_posts.is_empty() || advancing {
            center
        } else {
            let flank = state.deposit_me.pos
                + Vec2::new(
                    -17.2 + (bot_state.id % 2) as f32 * 1.05,
                    -4.6 + (bot_state.id / 2 % 3) as f32 * 1.05,
                );
            if bot_state.id % 6 == 0 && point_free(conf, flank) {
                flank
            } else {
                home_posts[bot_state.id.saturating_sub(8) as usize % home_posts.len()]
            }
        };
        let contester = recovery.is_none()
            && advancing
            && (finishing || profile.guards == 32 || guards & (1 << bot_state.id) != 0);
        bot_action.move_action =
            spaced_move(state, conf, bot_state, home, contester, profile.spacing);

        let enemy = state
            .fleet_other
            .iter()
            .filter(|enemy| {
                enemy.pos.dist(&if advancing {
                    center
                } else {
                    state.deposit_me.pos
                }) <= profile.leash
            })
            .min_by_key(|enemy| {
                let exposed = clear_shot(state, conf, bot_state.pos, enemy.pos)
                    && bot_state.pos.dist(&enemy.pos) <= conf.bot.blaster_range
                    && enemy.invulnerable_until_tick <= state.tick
                    && covered & (1 << enemy.id) == 0;
                (
                    !exposed,
                    std::cmp::Reverse(if exposed {
                        match profile.targeting.as_str() {
                            "healers" => u32::from(enemy.class() == BotClass::Healer) * 10,
                            "weakest" => (100.0 * (conf.bot.health - enemy.health).max(0.0)) as u32,
                            "miners" => u32::from(enemy.class() == BotClass::Extractor) * 10,
                            "splash" => state
                                .fleet_other
                                .iter()
                                .filter(|other| {
                                    other.pos.dist(&enemy.pos)
                                        <= conf.bot.base_blaster_splash_radius
                                        && other.invulnerable_until_tick <= state.tick
                                })
                                .count() as u32,
                            _ => 0,
                        }
                    } else {
                        0
                    }),
                    (bot_state.pos.dist_sq(&enemy.pos) * 1000.0) as u32,
                )
            });
        let enemy = match enemy {
            Some(enemy) => enemy,
            None => continue,
        };
        let goal = if let Some(rear) = recovery {
            rear
        } else if !advancing {
            home
        } else {
            firing_position(
                state,
                conf,
                bot_state.pos,
                enemy.pos,
                contester,
                profile.range,
            )
        };
        bot_action.move_action =
            spaced_move(state, conf, bot_state, goal, contester, profile.spacing);
        if profile.strafe && !contester && bot_state.pos.dist(&enemy.pos) < conf.bot.blaster_range {
            let tangent = (enemy.pos - bot_state.pos)
                .normalize_or_zero()
                .rotate_deg(90.0)
                * if (state.tick / 80 + bot_state.id as u32) % 2 == 0 {
                    0.7
                } else {
                    -0.7
                };
            let mut dodge = move_bot(bot_action.move_action.direction + tangent);
            dodge.sanitize();
            let next = bot_state.pos + dodge.direction * conf.bot.speed;
            if point_free(conf, next)
                && obstacles(state, conf)
                    .iter()
                    .all(|&(p, r)| next.dist(&p) >= r + conf.bot.radius)
            {
                bot_action.move_action = dodge;
            }
        }
        bot_action.move_action.sanitize();
        let mut aim = enemy.pos + enemy.vel;
        bot_action.turn_action = turn_towards(aim);
        let origin = bot_state.pos + bot_action.move_action.direction * conf.bot.speed;
        let desired = (aim - origin).angle_deg();
        let angle = bot_state.angle
            + diff_degrees(desired, bot_state.angle)
                .clamp(-conf.bot.turn_speed, conf.bot.turn_speed);
        let mut hits = shot_targets(state, conf, origin, angle) & !covered;
        if profile.healing_ball && hits == 0 && bot_state.next_fire_tick() <= state.tick {
            for other in state.fleet_other.iter() {
                let candidate = other.pos + other.vel;
                let desired = (candidate - origin).angle_deg();
                let angle = bot_state.angle
                    + diff_degrees(desired, bot_state.angle)
                        .clamp(-conf.bot.turn_speed, conf.bot.turn_speed);
                let available = shot_targets(state, conf, origin, angle) & !covered;
                if available.count_ones() > hits.count_ones() {
                    hits = available;
                    aim = candidate;
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

    action.rush_order =
        !state.in_endgame(conf) && state.fabricator_me.tokens >= conf.fabricator.rush_cost;

    action
}

fn payload_guards(state: &GameState, conf: &GameConfig, count: usize) -> u32 {
    let payload = state.payload_pos();
    let mut candidates: Vec<_> = state
        .fleet_me
        .iter()
        .filter(|bot| bot.class() == BotClass::Battle)
        .filter_map(|bot| {
            let distance = if bot.pos.dist(&payload) <= conf.payload.capture_radius {
                Some(0.0)
            } else {
                path_length(conf, bot.pos, payload)
            };
            distance.map(|distance| (bot.id, distance))
        })
        .collect();
    candidates.sort_by(|(a_id, a), (b_id, b)| a.total_cmp(b).then(a_id.cmp(b_id)));
    candidates
        .into_iter()
        .take(count)
        .fold(0, |mask, (id, _)| mask | (1 << id))
}

fn spaced_move(
    state: &GameState,
    conf: &GameConfig,
    bot: &BotState,
    goal: Vec2,
    guard: bool,
    strength: f32,
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
            let angle = bot.id.min(ally.id) as f32 * 137.5 + bot.id.max(ally.id) as f32 * 37.0;
            Vec2::from_angle_deg(angle) * if bot.id < ally.id { 1.0 } else { -1.0 }
        };
        push += away * ((spacing - distance) / spacing);
    }
    let mut spread = move_bot(travel.direction + push * strength);
    spread.sanitize();
    let next = bot.pos + spread.direction * conf.bot.speed;
    let payload = state.payload_pos();
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
    engagement_range: f32,
) -> Vec2 {
    let payload = state.payload_pos();
    let in_position = !guard || from.dist(&payload) <= conf.payload.capture_radius - conf.bot.speed;
    if in_position
        && from.dist(&target) <= conf.bot.blaster_range * engagement_range
        && clear_shot(state, conf, from, target)
    {
        return from;
    }
    let center = if guard { payload } else { target };
    let radius = if guard {
        conf.payload.radius + conf.bot.radius + 0.15
    } else {
        conf.bot.blaster_range * engagement_range
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
    fn roster_changes_orders_and_composition() {
        let conf = GameConfig {
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

        init_topology(&conf.map, conf.bot.radius);
        let profiles = crate::roster().unwrap();
        assert_eq!(profiles.len(), 101);
        assert!(profiles[..43]
            .iter()
            .all(|p| !p.coordinated && p.medic_leash == 0.0 && !p.finish_push));
        let profile = |name: &str| profiles.iter().find(|p| p.name == name).unwrap();
        assert!(profiles[..87].iter().all(|p| !p.evade_miners));
        let mut miner_state = GameState::new(&conf);
        let miner = miner_state.fleet_me.add();
        miner_state.fleet_me[miner].special = SpecialState::new(BotClass::Extractor);
        miner_state.fleet_me[miner].pos = Vec2::new(10.0, 10.0);
        let threat = miner_state.fleet_other.add();
        miner_state.fleet_other[threat].pos = Vec2::new(10.0, 12.0);
        let escape = battle_strategy(&miner_state, &conf, profile("approx-iamabot-escort"));
        assert!(escape.bots[miner as usize].move_action.direction.y < 0.0,
            "retreating-miner profile must move away from an approaching fighter");
        let normal = battle_strategy(&miner_state, &conf, profile("approx-janice-escort"));
        assert!(normal.bots[miner as usize].move_action.direction.y > 0.0,
            "existing profiles must still move toward the mining spot");

        let mut state = GameState::new(&conf);
        let bot = state.fleet_me.add();
        state.fleet_me[bot].special = SpecialState::new(BotClass::Battle);
        state.fleet_me[bot].pos = Vec2::new(10.0, 16.0);
        assert_eq!(
            battle_strategy(&state, &conf, profile("zero-miner-allin")).fabricator_next,
            BotClass::Battle
        );
        assert_eq!(
            battle_strategy(&state, &conf, profile("eight-miner-factory")).fabricator_next,
            BotClass::Extractor
        );
        assert_eq!(
            battle_strategy(&state, &conf, profile("healer-only-stress")).fabricator_next,
            BotClass::Healer
        );
        for _ in 0..4 {
            let id = state.fleet_me.add();
            state.fleet_me[id].special = SpecialState::new(BotClass::Battle);
            state.fleet_me[id].pos = Vec2::new(10.0, 16.0);
        }
        for _ in 0..2 {
            let id = state.fleet_me.add();
            state.fleet_me[id].special = SpecialState::new(BotClass::Extractor);
        }
        assert_eq!(
            battle_strategy(&state, &conf, profile("healer-swarm")).fabricator_next,
            BotClass::Healer
        );
        let mut transition = profile("greed-then-army").clone();
        transition.switch_tick = 100;
        assert_eq!(
            battle_strategy(&state, &conf, &transition).fabricator_next,
            BotClass::Extractor
        );
        state.tick = 100;
        assert_ne!(
            battle_strategy(&state, &conf, &transition).fabricator_next,
            BotClass::Extractor
        );
        assert_eq!(payload_guards(&state, &conf, 1).count_ones(), 1);
        assert_eq!(payload_guards(&state, &conf, 2).count_ones(), 2);
        assert_eq!(payload_guards(&state, &conf, 32).count_ones(), 5);
        let close = spaced_move(
            &state,
            &conf,
            &state.fleet_me[0],
            state.payload_pos(),
            false,
            0.0,
        );
        let wide = spaced_move(
            &state,
            &conf,
            &state.fleet_me[0],
            state.payload_pos(),
            false,
            5.0,
        );
        assert!(
            close.direction.dist(&wide.direction) > 0.01,
            "spacing profile must change orders"
        );
        let mut raid = GameState::new(&conf);
        raid.fleet_me.add();
        raid.fleet_me[0].pos = Vec2::new(10.0, 16.0);
        let orders = battle_strategy(&raid, &conf, profile("economy-base-raider"));
        assert!(
            orders.bots[0].move_action.direction.y < 0.0,
            "base raiders must head toward the enemy deposit before enemies are visible"
        );
        // A new profile must reproduce the observed opening without changing old profiles.
        assert!(profiles[..32].iter().all(|p| !p.healing_ball));
        let janice = profile("approx-janice-escort");
        let mut opening = GameState::new(&conf);
        for tick in 0..17 {
            opening.tick = tick;
            let class = battle_strategy(&opening, &conf, janice).fabricator_next;
            let id = opening.fleet_me.add();
            opening.fleet_me[id].special = SpecialState::new(class);
        }
        assert_eq!(
            opening
                .fleet_me
                .iter()
                .filter(|b| b.class() == BotClass::Extractor)
                .count(),
            3
        );
        assert_eq!(
            opening
                .fleet_me
                .iter()
                .filter(|b| b.class() == BotClass::Battle)
                .count(),
            10
        );
        assert_eq!(
            opening
                .fleet_me
                .iter()
                .filter(|b| b.class() == BotClass::Healer)
                .count(),
            4
        );
        assert_eq!(opening.fleet_me[4].class(), BotClass::Healer);
        assert_eq!(opening.fleet_me[8].class(), BotClass::Extractor);
        let mut wounded = GameState::new(&conf);
        wounded.tick = 1000;
        wounded.fleet_me.add();
        wounded.fleet_me[0].pos = Vec2::new(10.0, 10.0);
        wounded.fleet_me[0].health = 4.0;
        let medic = wounded.fleet_me.add();
        wounded.fleet_me[medic].pos = Vec2::new(7.0, 10.0);
        wounded.fleet_me[medic].special = SpecialState::new(BotClass::Healer);
        wounded.fleet_other.add();
        wounded.fleet_other[0].pos = Vec2::new(14.0, 10.0);
        let action = battle_strategy(&wounded, &conf, janice);
        assert!(
            action.bots[0].move_action.direction.x < 0.0,
            "wounded escort returns to healing support"
        );
        assert!(
            matches!(
                action.bots[0].special_action,
                SpecialAction::Battle { fire: true }
            ),
            "retreat retains legal shots"
        );
        assert!(profiles[..35].iter().all(|p| !p.mobilize));
        let gang = profile("approx-gang-late-escort");
        let mut opening = GameState::new(&conf);
        for tick in 0..17 {
            opening.tick = tick;
            let class = battle_strategy(&opening, &conf, gang).fabricator_next;
            let id = opening.fleet_me.add();
            opening.fleet_me[id].special = SpecialState::new(class);
        }
        for (class, count) in [
            (BotClass::Extractor, 8),
            (BotClass::Battle, 8),
            (BotClass::Healer, 1),
        ] {
            assert_eq!(
                opening
                    .fleet_me
                    .iter()
                    .filter(|b| b.class() == class)
                    .count(),
                count
            );
        }
        opening.tick = gang.switch_tick - 1;
        assert!(!battle_strategy(&opening, &conf, gang).bots[0].self_destruct);
        opening.tick = gang.switch_tick;
        let orders = battle_strategy(&opening, &conf, gang);
        assert_eq!(orders.bots.iter().filter(|b| b.self_destruct).count(), 8);
        assert_ne!(orders.fabricator_next, BotClass::Extractor);
        assert!(
            orders.rush_order,
            "mobilization must occur before paid builds close"
        );
        let attacker = opening.fleet_other.add();
        opening.fleet_other[attacker].pos = opening.fleet_me[0].pos;
        assert!(
            !battle_strategy(&opening, &conf, gang).bots[0].self_destruct,
            "do not retire miners while a simultaneous shot could kill them"
        );
        opening.fleet_other.remove(attacker);
        opening.fleet_me[8].pos = Vec2::new(15.0, 26.0);
        opening.tick = gang.delay_push - 1;
        let hold = battle_strategy(&opening, &conf, gang).bots[8]
            .move_action
            .direction;
        opening.tick = gang.delay_push;
        let advance = battle_strategy(&opening, &conf, gang).bots[8]
            .move_action
            .direction;
        assert!(hold.dot(state.deposit_me.pos - opening.fleet_me[8].pos) > 0.0);
        assert!(advance.dot(opening.payload_pos() - opening.fleet_me[8].pos) > 0.0);
        assert!(
            hold.dist(&advance) > 0.1,
            "delayed army must switch from home to payload"
        );
        let mut newer = GameState::new(&conf);
        let gang13 = profile("approx-gang13-escort");
        for tick in 0..17 {
            newer.tick = tick;
            let class = battle_strategy(&newer, &conf, gang13).fabricator_next;
            let id = newer.fleet_me.add();
            newer.fleet_me[id].special = SpecialState::new(class);
        }
        for (class, count) in [
            (BotClass::Extractor, 8),
            (BotClass::Battle, 6),
            (BotClass::Healer, 3),
        ] {
            assert_eq!(
                newer.fleet_me.iter().filter(|b| b.class() == class).count(),
                count
            );
        }
        for bot in newer.fleet_me.iter_mut() {
            bot.pos = Vec2::new(15.0, 26.0);
            bot.health = conf.bot.health;
        }
        let mut stage_profile = gang13.clone();
        stage_profile.spacing = 0.0;
        let gang13 = &stage_profile;
        newer.tick = gang13.delay_push;
        let staged = battle_strategy(&newer, &conf, gang13);
        assert!(
            staged.bots[8].move_action.direction.y < 0.0,
            "send a guard to contest early"
        );
        assert!(
            staged.bots[10].move_action.direction.y > 0.0,
            "keep the reserve near home until mobilization"
        );
        newer.tick = gang13.switch_tick;
        assert!(
            battle_strategy(&newer, &conf, gang13).bots[10]
                .move_action
                .direction
                .y
                < 0.0,
            "mobilization releases the reserve"
        );
        assert!(profiles[..38]
            .iter()
            .all(|p| p.opening_healers.is_none() && !p.staged_push));
        // Healthy reinforcements wait for nearby healing instead of outrunning support.
        let supported = profile("team-name-supported");
        wounded.fleet_me[0].health = conf.bot.health;
        wounded.fleet_me[0].pos = Vec2::new(12.0, 10.0);
        wounded.fleet_me[medic].pos = Vec2::new(6.0, 10.0);
        assert!(
            battle_strategy(&wounded, &conf, supported).bots[0]
                .move_action
                .direction
                .x
                < 0.0
        );
        let mut released = supported.clone();
        released.finish_push = true;
        wounded.capture = 0.9;
        assert!(
            battle_strategy(&wounded, &conf, &released).bots[0]
                .move_action
                .direction
                .x
                > 0.0,
            "finish-push mode releases the medic leash near delivery"
        );
        let mut triage = GameState::new(&conf);
        let h = triage.fleet_me.add();
        triage.fleet_me[h].special = SpecialState::new(BotClass::Healer);
        triage.fleet_me[h].pos = Vec2::new(10.0, 10.0);
        triage.fleet_me[h].angle = 0.0;
        for (x, hp) in [(12.0, 7.0), (8.0, 5.0)] {
            let id = triage.fleet_me.add();
            triage.fleet_me[id].pos = Vec2::new(x, 10.0);
            triage.fleet_me[id].health = hp;
        }
        assert!(matches!(
            battle_strategy(&triage, &conf, supported).bots[h as usize].special_action,
            SpecialAction::Healer {
                fire: true,
                target: 1
            }
        ));
        triage.fleet_me[2].health = 2.0;
        assert!(matches!(
            battle_strategy(&triage, &conf, supported).bots[h as usize].special_action,
            SpecialAction::Healer {
                fire: true,
                target: 2
            }
        ));
        let mut escort_test = GameState::new(&conf);
        for (class, pos) in [
            (BotClass::Healer, Vec2::new(10.0, 10.0)),
            (BotClass::Healer, Vec2::new(10.0, 10.1)),
            (BotClass::Battle, Vec2::new(12.0, 10.0)),
            (BotClass::Battle, Vec2::new(12.0, 10.2)),
        ] {
            let id = escort_test.fleet_me.add();
            escort_test.fleet_me[id].special = SpecialState::new(class);
            escort_test.fleet_me[id].pos = pos;
            escort_test.fleet_me[id].health = conf.bot.health;
        }
        let orders = battle_strategy(&escort_test, &conf, supported);
        assert!(matches!(
            orders.bots[0].special_action,
            SpecialAction::Healer {
                fire: false,
                target: 2
            }
        ));
        assert!(
            matches!(
                orders.bots[1].special_action,
                SpecialAction::Healer {
                    fire: false,
                    target: 3
                }
            ),
            "idle medics distribute escorts instead of following one fighter"
        );
        for p in &profiles {
            let action = battle_strategy(&state, &conf, p);
            for bot in state.fleet_me.iter() {
                assert!(action.bots[bot.id as usize]
                    .move_action
                    .direction
                    .norm()
                    .is_finite());
            }
        }
    }
}
