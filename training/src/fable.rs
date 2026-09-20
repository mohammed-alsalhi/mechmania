// Generated independently by claude-fable-5-1; only API compatibility edits before evaluation.
use crate::core::*;

// Independent sparring bot: everything orbits the payload. One anchor always sits inside the
// capture disc (behind the payload, using its hull as cover), a leashed front line engages
// anything that can threaten the capture zone, healers hide behind the payload and heal
// through it, and wounded fighters rotate back to the medics instead of trading down.

/// Does the segment a->b pass through a disc? (navigate_to only knows walls, not solid discs.)
fn seg_hits_disc(a: Vec2, b: Vec2, c: Vec2, r: f32) -> bool {
    let ab = b - a;
    let l2 = ab.norm_sq();
    let t = if l2 <= 1e-9 {
        0.0
    } else {
        ((c - a).dot(ab) / l2).clamp(0.0, 1.0)
    };
    (a + ab * t).dist_sq(&c) < r * r
}

/// One-tick move direction toward `to`: skirt a nearby solid disc, otherwise wall-aware nav.
fn go(conf: &GameConfig, from: Vec2, to: Vec2, discs: &[(Vec2, f32)]) -> Vec2 {
    if from.dist_sq(&to) < 0.01 {
        return Vec2::ZERO;
    }
    for &(c, r) in discs {
        let rr = r + conf.bot.radius + 0.1;
        if from.dist(&c) < rr + 1.5 && seg_hits_disc(from, to, c, rr) {
            let away = (from - c).normalize_or_zero();
            let mut t = away.rotate_deg(90.0);
            if t.dot(to - from) < 0.0 {
                t = t * -1.0;
            }
            return (t + away * 0.3).normalize_or_zero();
        }
    }
    navigate_to(conf, from, to)
}

/// Will this bot face `aim` within `tol_deg` after this tick's turn resolves?
fn aim_ok(bot: &BotState, aim: Vec2, turn: f32, tol_deg: f32) -> bool {
    let want = (aim - bot.pos).angle_deg();
    (diff_degrees(bot.angle, want).abs() - turn).max(0.0) <= tol_deg
}

fn nearest<'a>(bots: &[&'a BotState], to: Vec2) -> Option<&'a BotState> {
    let mut best: Option<(&'a BotState, f32)> = None;
    for &b in bots {
        let d = b.pos.dist_sq(&to);
        if best.map_or(true, |(_, bd)| d < bd) {
            best = Some((b, d));
        }
    }
    best.map(|(b, _)| b)
}

pub fn battle_strategy(state: &GameState, conf: &GameConfig) -> FleetAction {
    let mut act = FleetAction::new();
    let tick = state.tick;
    let bc = &conf.bot;
    let max_hp = bc.health;
    let range = bc.blaster_range;
    let heal_r = bc.base_heal_range;
    let p = state.payload_pos();
    let pay_r = conf.payload.radius;
    let cap_r = conf.payload.capture_radius;
    let dep = state.deposit_me.pos;
    let edep = state.deposit_other.pos;
    let dep_r = conf.deposit.radius;
    let discs = [(p, pay_r), (dep, dep_r), (edep, dep_r)];

    let mine: Vec<&BotState> = state.fleet_me.iter().collect();
    let enemies: Vec<&BotState> = state
        .fleet_other
        .iter()
        .filter(|e| e.invulnerable_until_tick <= tick)
        .collect();
    let fighters: Vec<&BotState> = mine
        .iter()
        .copied()
        .filter(|b| matches!(b.class(), BotClass::Battle))
        .collect();
    let healers: Vec<&BotState> = mine
        .iter()
        .copied()
        .filter(|b| matches!(b.class(), BotClass::Healer))
        .collect();
    let miners: Vec<&BotState> = mine
        .iter()
        .copied()
        .filter(|b| matches!(b.class(), BotClass::Extractor))
        .collect();
    let (n_f, n_h, n_m) = (fighters.len(), healers.len(), miners.len());

    // ---- production: 3 miners, then ~2 healers per 5 fighters; rush whenever affordable ----
    let want_m = 3usize.min(conf.deposit.extractor_cap as usize);
    act.fabricator_next = if n_m < want_m {
        BotClass::Extractor
    } else if n_h * 5 < n_f * 2 {
        BotClass::Healer
    } else {
        BotClass::Battle
    };
    act.rush_order = !state.in_endgame(conf)
        && !state.fleet_me.is_full()
        && state.fabricator_me.tokens >= conf.fabricator.rush_cost;

    // ---- where is the fight coming from ----
    let engage = cap_r + range;
    let threats: Vec<&BotState> = enemies
        .iter()
        .copied()
        .filter(|e| e.pos.dist(&p) <= engage)
        .collect();
    let tdir = {
        let raw = if !threats.is_empty() {
            let sum = threats.iter().fold(Vec2::ZERO, |a, e| a + e.pos);
            sum * (1.0 / threats.len() as f32) - p
        } else if let Some(e) = nearest(&enemies, p) {
            e.pos - p
        } else {
            edep - p
        };
        let d = raw.normalize_or_zero();
        if d.norm_sq() < 0.5 {
            (edep - dep).normalize_or_zero()
        } else {
            d
        }
    };
    let perp = tdir.rotate_deg(90.0);
    let leash = cap_r + range * 0.5;
    let anchor_id = nearest(&fighters, p)
        .or_else(|| nearest(&healers, p))
        .map(|b| b.id);
    // Healers rally at the payload once the army is there, else on the army itself.
    let home = if fighters.is_empty() || fighters.iter().any(|f| f.pos.dist(&p) < cap_r * 2.0) {
        p
    } else {
        fighters.iter().fold(Vec2::ZERO, |a, f| a + f.pos) * (1.0 / n_f as f32)
    };

    // ---- fighters ----
    let mut claimed = [0u8; BOTS_MAX as usize];
    let vel_cap = bc.speed * bc.speed * 4.0;
    let mut slot = 0usize;
    for f in &fighters {
        let ready = f.next_fire_tick() <= tick;
        // Target: in range, clear ray, prefer enemies nobody else hits this tick, then low HP.
        let mut best: Option<(&BotState, Vec2, f32)> = None;
        for e in &enemies {
            let ep = if e.vel.norm_sq() <= vel_cap {
                e.pos + e.vel
            } else {
                e.pos
            };
            let d = f.pos.dist(&ep);
            if d > range || !has_line_of_sight(conf, f.pos, ep) {
                continue;
            }
            if discs.iter().any(|&(c, r)| seg_hits_disc(f.pos, ep, c, r)) {
                continue;
            }
            let score = claimed[e.id as usize] as f32 * 100.0 + e.health + d * 0.1;
            if best.map_or(true, |(_, _, s)| score < s) {
                best = Some((e, ep, score));
            }
        }
        let a = &mut act.bots[f.id as usize];
        a.special_action = SpecialAction::Battle { fire: false };
        match best {
            Some((e, ep, _)) => {
                a.turn_action = turn_towards(ep);
                let tol = (bc.radius * 0.7 / f.pos.dist(&ep).max(0.5))
                    .atan()
                    .to_degrees();
                if ready && aim_ok(f, ep, bc.turn_speed, tol) {
                    a.special_action = SpecialAction::Battle { fire: true };
                    claimed[e.id as usize] += 1;
                }
            }
            None => a.turn_action = turn_towards(f.pos + tdir),
        }

        let is_anchor = anchor_id == Some(f.id);
        let medic = nearest(&healers, f.pos).filter(|h| {
            f.health < max_hp * 0.4 || (f.health < max_hp * 0.75 && f.pos.dist(&h.pos) < heal_r)
        });
        let front = {
            let (col, row) = ((slot % 10) as f32, (slot / 10) as f32);
            p + tdir * (cap_r * 0.8 - row * 0.4) + perp * ((col - 4.5) * 0.6)
        };
        let dest = if is_anchor {
            p - tdir * (pay_r + 0.6)
        } else if let Some(h) = medic {
            h.pos - tdir * 0.4
        } else if f.pos.dist(&p) > leash {
            front
        } else if best.is_some() {
            f.pos
        } else if let Some(t) = nearest(&threats, f.pos) {
            t.pos
        } else {
            front
        };
        if !is_anchor {
            slot += 1;
        }
        a.move_action = move_bot(go(conf, f.pos, dest, &discs));
    }

    // ---- healers: hide behind the payload, heal through it, follow the most wounded ----
    let mut healing = [0u8; BOTS_MAX as usize];
    let back_line = home - tdir * (pay_r + 0.5);
    for (k, h) in healers.iter().enumerate() {
        let mut best: Option<(&BotState, f32)> = None;
        for b in &mine {
            if b.id == h.id || b.health >= max_hp - 1e-3 || healing[b.id as usize] >= 2 {
                continue;
            }
            let score = b.health / max_hp
                + h.pos.dist(&b.pos) * 0.04
                + healing[b.id as usize] as f32 * 0.25;
            if best.map_or(true, |(_, s)| score < s) {
                best = Some((b, score));
            }
        }
        let back = back_line + perp * ((k as f32 - (n_h as f32 - 1.0) / 2.0) * 0.5);
        let a = &mut act.bots[h.id as usize];
        match best {
            Some((b, _)) => {
                let ok =
                    h.pos.dist(&b.pos) <= heal_r * 0.95 && has_line_of_sight(conf, h.pos, b.pos);
                if ok {
                    healing[b.id as usize] += 1;
                }
                let mut dest = if ok {
                    h.pos
                } else {
                    b.pos - tdir * (heal_r * 0.6)
                };
                if dest.dist(&home) > leash {
                    dest = back;
                }
                a.special_action = SpecialAction::Healer {
                    fire: ok,
                    target: b.id,
                };
                a.turn_action = turn_towards(b.pos);
                a.move_action = move_bot(go(conf, h.pos, dest, &discs));
            }
            None => {
                let buddy = mine.iter().find(|b| b.id != h.id).map_or(h.id, |b| b.id);
                a.special_action = SpecialAction::Healer {
                    fire: false,
                    target: buddy,
                };
                a.turn_action = turn_towards(h.pos + tdir);
                a.move_action = move_bot(go(conf, h.pos, back, &discs));
            }
        }
    }

    // ---- miners: hug the deposit on the side away from danger (its hull blocks shots) ----
    for (k, m) in miners.iter().enumerate() {
        let threat = nearest(&enemies, m.pos).filter(|e| e.pos.dist(&dep) < range + dep_r + 1.0);
        let base = match threat {
            Some(e) => dep - e.pos,
            None => dep - edep,
        }
        .normalize_or_zero();
        let dir = base.rotate_deg((k as f32 - (n_m as f32 - 1.0) / 2.0) * 45.0);
        let dest = dep + dir * (dep_r + conf.bot.radius + 0.15);
        let a = &mut act.bots[m.id as usize];
        a.special_action = SpecialAction::Extractor { mine: true };
        a.turn_action = turn_towards(dep);
        a.move_action = move_bot(go(conf, m.pos, dest, &discs));
    }

    act
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_blocks_ray_through_it_but_not_a_tangent() {
        let c = Vec2::new(5.0, 5.0);
        assert!(seg_hits_disc(
            Vec2::new(0.0, 5.0),
            Vec2::new(10.0, 5.0),
            c,
            0.75
        ));
        assert!(!seg_hits_disc(
            Vec2::new(0.0, 6.0),
            Vec2::new(10.0, 6.0),
            c,
            0.75
        ));
        assert!(!seg_hits_disc(
            Vec2::new(0.0, 5.0),
            Vec2::new(4.0, 5.0),
            c,
            0.75
        ));
    }
}
