// Generated independently through Cursor using cursor-grok-4.6-high; API compatibility edits only.
use crate::core::*;

fn finite(v: Vec2) -> Vec2 {
    if v.x.is_finite() && v.y.is_finite() {
        v
    } else {
        Vec2::ZERO
    }
}

fn step_pos(pos: Vec2, dir: Vec2, speed: f32) -> Vec2 {
    pos + finite(dir).normalize_or_zero() * speed
}

fn needs_retreat(hp: f32, dmg: f32, max_hp: f32) -> bool {
    hp <= dmg + 0.05 || hp < max_hp * 0.42
}

fn push_disc(p: Vec2, c: Vec2, min_d: f32) -> Vec2 {
    let d = p.dist(&c);
    if d >= min_d {
        return p;
    }
    let dir = (p - c).normalize_or_zero();
    let dir = if dir.norm_sq() < 1e-10 {
        Vec2::new(-0.75, -0.75)
    } else {
        dir
    };
    c + dir * min_d
}

fn nav_to(conf: &GameConfig, state: &GameState, from: Vec2, mut to: Vec2, miner: bool) -> Vec2 {
    let pr = conf.payload.radius + conf.bot.radius + 0.07;
    to = push_disc(to, state.payload_pos(), pr);
    if !miner {
        let dr = conf.deposit.radius + conf.bot.radius + 0.07;
        to = push_disc(to, state.deposit_me.pos, dr);
        to = push_disc(to, state.deposit_other.pos, dr);
    }
    let dir = finite(navigate_to(conf, from, to));
    let nxt = step_pos(from, dir, conf.bot.speed);
    if nxt.dist(&state.payload_pos()) < pr {
        let tangent = (from - state.payload_pos())
            .rotate_deg(80.0)
            .normalize_or_zero();
        let alt = state.payload_pos() + tangent * (pr + 0.28);
        return finite(navigate_to(conf, from, alt));
    }
    dir
}

fn in_face(ang: f32, look: f32, arc: f32) -> bool {
    diff_degrees(ang, look).abs() <= arc * 0.5 + 0.75
}

fn hull_hit(from: Vec2, aim: Vec2, enemies: &[&BotState], range: f32, rad: f32) -> Option<u8> {
    let dir = (aim - from).normalize_or_zero();
    if dir.norm_sq() < 1e-12 {
        return None;
    }
    let mut best: Option<(f32, u8)> = None;
    for e in enemies {
        let rel = e.pos - from;
        let along = rel.dot(dir);
        if along <= 0.05 || along > range {
            continue;
        }
        let perp = (rel - dir * along).norm();
        if perp <= rad * 2.05 && best.map(|(a, _)| along < a).unwrap_or(true) {
            best = Some((along, e.id));
        }
    }
    best.map(|(_, id)| id)
}

pub fn battle_strategy(state: &GameState, conf: &GameConfig) -> FleetAction {
    let mut action = FleetAction::new();
    let me: Vec<&BotState> = state.fleet_me.iter().collect();
    let them: Vec<&BotState> = state.fleet_other.iter().collect();
    let pay = state.payload_pos();
    let speed = conf.bot.speed;
    let dmg = conf.bot.blaster_damage;
    let max_hp = conf.bot.health;
    let range = conf.bot.blaster_range;
    let turn = conf.bot.turn_speed;
    let cap_r = conf.payload.capture_radius;
    let br = conf.bot.radius;
    let heal_r = conf.bot.base_heal_range;
    let end = state.in_endgame(conf);
    let arc = 90.0;

    let mut n_ext = 0usize;
    let mut n_heal = 0usize;
    let mut n_bat = 0usize;
    for b in &me {
        match b.class() {
            BotClass::Extractor => n_ext += 1,
            BotClass::Healer => n_heal += 1,
            BotClass::Battle => n_bat += 1,
        }
    }

    let us_cap = me.iter().filter(|b| b.pos.dist(&pay) <= cap_r).count();
    let them_cap = them.iter().filter(|b| b.pos.dist(&pay) <= cap_r).count();
    let turtle = state.capture > 0.52 && them_cap == 0;
    let losing = state.capture < -0.12;

    let mut home_d = (state.deposit_me.pos - pay).normalize_or_zero();
    if home_d.norm_sq() < 1e-8 {
        home_d = Vec2::new(-1.0, -1.0).normalize_or_zero();
    }
    let mut enemy_d = (state.deposit_other.pos - pay).normalize_or_zero();
    if enemy_d.norm_sq() < 1e-8 {
        enemy_d = home_d * -1.0;
    }

    let hold_r = (conf.payload.radius + br + 0.38).clamp(
        conf.payload.radius + br + 0.14,
        (cap_r - 0.18).max(conf.payload.radius + br + 0.14),
    );
    let intercept = pay + enemy_d * hold_r;
    let home_rim = pay + home_d * hold_r;
    let pocket = pay + home_d * (hold_r + 1.15).min(cap_r + 0.9);

    let mut hsum = Vec2::ZERO;
    let mut hc = 0.0f32;
    for b in &me {
        if matches!(b.class(), BotClass::Healer) {
            hsum = hsum + b.pos;
            hc += 1.0;
        }
    }
    let hcen = if hc > 0.0 { hsum / hc } else { pocket };

    let heal_threat = them.iter().copied().min_by(|a, b| {
        a.pos
            .dist(&hcen)
            .partial_cmp(&b.pos.dist(&hcen))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let threatened = heal_threat
        .map(|e| {
            me.iter()
                .any(|h| matches!(h.class(), BotClass::Healer) && e.pos.dist(&h.pos) < range * 0.9)
        })
        .unwrap_or(false);

    let mut battles: Vec<&BotState> = me
        .iter()
        .copied()
        .filter(|b| matches!(b.class(), BotClass::Battle))
        .collect();
    battles.sort_by(|a, b| {
        let wa = needs_retreat(a.health, dmg, max_hp) as u8;
        let wb = needs_retreat(b.health, dmg, max_hp) as u8;
        wa.cmp(&wb).then_with(|| {
            a.pos
                .dist(&intercept)
                .partial_cmp(&b.pos.dist(&intercept))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let hold_n = if them_cap > 0 || losing {
        2.min(battles.len()).max(1.min(battles.len()))
    } else if turtle || battles.len() >= 5 {
        2.min(battles.len())
    } else {
        1.min(battles.len())
    };
    let mut hold_mask: u32 = 0;
    for b in battles.iter().take(hold_n) {
        hold_mask |= 1u32 << (b.id as u32);
    }
    let holders_healthy = battles
        .iter()
        .take(hold_n)
        .any(|b| !needs_retreat(b.health, dmg, max_hp));

    let budgeted = get_budget().remaining > 0;
    let mut fire_at = [None::<u8>; 32];
    let mut aim_pos = [None::<Vec2>; 32];
    let mut claimed = [false; 32];
    let mut shooters: Vec<&BotState> = battles.clone();
    shooters.sort_by(|a, b| {
        let ra = (a.next_fire_tick() <= state.tick) as u8;
        let rb = (b.next_fire_tick() <= state.tick) as u8;
        rb.cmp(&ra).then_with(|| a.id.cmp(&b.id))
    });

    let mut targs: Vec<&BotState> = them
        .iter()
        .copied()
        .filter(|e| state.tick >= e.invulnerable_until_tick)
        .collect();
    targs.sort_by(|a, b| {
        let score = |e: &BotState| {
            let mut s = 0.0;
            let cap = e.pos.dist(&pay) <= cap_r;
            let near_h = e.pos.dist(&hcen) < range + 1.5;
            if us_cap == 0 || them_cap > 0 {
                if cap {
                    s += 140.0;
                }
            } else if matches!(e.class(), BotClass::Healer) {
                s += 85.0;
            }
            if near_h {
                s += 90.0;
            }
            if cap {
                s += 35.0;
            }
            if matches!(e.class(), BotClass::Healer) {
                s += 30.0;
            }
            s += (max_hp - e.health) * 4.0;
            s -= e.pos.dist(&pay) * 0.35;
            s
        };
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for sh in &shooters {
        let mut picked: Option<(u8, Vec2)> = None;
        for t in &targs {
            if claimed[t.id as usize] {
                continue;
            }
            let aim = t.pos + t.vel;
            let hit = hull_hit(sh.pos, aim, &them, range, br).unwrap_or(t.id);
            if claimed[hit as usize] {
                continue;
            }
            let hp = them
                .iter()
                .find(|e| e.id == hit)
                .map(|e| e.pos)
                .unwrap_or(aim);
            if sh.pos.dist(&hp) > range + speed * 2.0 {
                continue;
            }
            picked = Some((hit, hp));
            break;
        }
        if let Some((id, p)) = picked {
            fire_at[sh.id as usize] = Some(id);
            aim_pos[sh.id as usize] = Some(p);
            for e in &them {
                if e.id == id || e.pos.dist(&p) < br * 6.0 {
                    claimed[e.id as usize] = true;
                }
            }
        } else if let Some(t) = targs.first() {
            aim_pos[sh.id as usize] = Some(t.pos);
        }
    }

    let mut ext_i = 0usize;
    let mut heal_i = 0usize;
    for b in &me {
        let id = b.id;
        let slot = &mut action.bots[id as usize];
        slot.self_destruct = false;
        let class = b.class();

        if matches!(class, BotClass::Extractor) {
            let mine_end = end && (losing || them_cap > 0 || state.capture < 0.35);
            if mine_end {
                let goal = home_rim + home_d.rotate_deg((id as f32) * 17.0) * 0.2;
                let dir = nav_to(conf, state, b.pos, goal, false);
                slot.move_action = move_bot(dir);
                slot.turn_action = turn_towards(intercept);
                slot.special_action = SpecialAction::Extractor { mine: false };
            } else {
                let face = (pay - state.deposit_me.pos).angle_deg();
                let ang = face + 120.0 * (ext_i as f32) - 120.0;
                ext_i += 1;
                let rad = conf.deposit.radius + br * 0.35;
                let goal = state.deposit_me.pos + Vec2::from_angle_deg(ang) * rad;
                let dir = nav_to(conf, state, b.pos, goal, true);
                slot.move_action = move_bot(dir);
                slot.turn_action = turn_towards(state.deposit_me.pos);
                slot.special_action = SpecialAction::Extractor { mine: true };
            }
            continue;
        }

        if matches!(class, BotClass::Healer) {
            let spread = (heal_i as f32 - 1.0) * 38.0;
            heal_i += 1;
            let slot_pos = pay + home_d.rotate_deg(spread) * hold_r;
            let wounded = me
                .iter()
                .copied()
                .filter(|a| a.id != id && a.health < max_hp - 0.04)
                .min_by(|a, c| {
                    let sa = a.health + a.pos.dist(&b.pos) * 0.2;
                    let sc = c.health + c.pos.dist(&b.pos) * 0.2;
                    sa.partial_cmp(&sc).unwrap_or(std::cmp::Ordering::Equal)
                });
            let danger = them.iter().any(|e| e.pos.dist(&b.pos) < range * 0.55);
            let mut goal = if danger {
                push_disc(b.pos + home_d * 1.4, pay, conf.payload.radius + br + 0.2)
            } else if let Some(w) = wounded {
                let back = w.pos + home_d * (heal_r * 0.55);
                if back.dist(&w.pos) > heal_r * 0.95 {
                    w.pos + home_d * 0.8
                } else {
                    back
                }
            } else {
                slot_pos
            };
            if b.health < max_hp * 0.55 {
                goal = state.deposit_me.pos * 0.45 + home_rim * 0.55;
            }
            let dir = nav_to(conf, state, b.pos, goal, false);
            let nxt = step_pos(b.pos, dir, speed);
            let look = wounded.map(|w| w.pos).unwrap_or(intercept);
            let want = (look - nxt).angle_deg();
            let nang = {
                let diff = diff_degrees(b.angle, want);
                normalize_degrees(b.angle + diff.clamp(-turn, turn))
            };
            let mut fire = false;
            let mut target = 0u8;
            if let Some(w) = wounded {
                let d = nxt.dist(&w.pos);
                if d <= heal_r
                    && d > 0.04
                    && in_face(nang, want, arc)
                    && has_line_of_sight(conf, nxt, w.pos)
                {
                    fire = true;
                    target = w.id;
                }
            }
            slot.move_action = move_bot(dir);
            slot.turn_action = turn_towards(look);
            slot.special_action = SpecialAction::Healer { fire, target };
            continue;
        }

        let is_hold = ((hold_mask >> (id as u32)) & 1) == 1;
        let peel = needs_retreat(b.health, dmg, max_hp)
            && n_heal > 0
            && !(is_hold && them_cap > 0 && us_cap <= 1 && !holders_healthy);
        let flank = if (id % 2) == 0 {
            enemy_d.rotate_deg(62.0)
        } else {
            enemy_d.rotate_deg(-62.0)
        };
        let mut goal = if peel {
            hcen
        } else if is_hold {
            intercept
        } else if threatened {
            heal_threat.map(|e| e.pos).unwrap_or(intercept)
        } else if turtle {
            pay + enemy_d * (cap_r * 0.72)
        } else if them_cap > 0 {
            them.iter()
                .copied()
                .filter(|e| e.pos.dist(&pay) <= cap_r + 1.2)
                .min_by(|a, c| {
                    a.pos
                        .dist(&b.pos)
                        .partial_cmp(&c.pos.dist(&b.pos))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|e| e.pos)
                .unwrap_or(intercept)
        } else {
            pay + flank.normalize_or_zero() * hold_r
        };
        if budgeted && is_hold && !point_free(conf, intercept) {
            goal = home_rim;
        }
        let dir = nav_to(conf, state, b.pos, goal, false);
        let nxt = step_pos(b.pos, dir, speed);
        let look = aim_pos[id as usize].unwrap_or(goal);
        let want = {
            let v = look - nxt;
            if v.norm_sq() < 1e-8 {
                b.angle
            } else {
                v.angle_deg()
            }
        };
        let nang = {
            let diff = diff_degrees(b.angle, want);
            normalize_degrees(b.angle + diff.clamp(-turn, turn))
        };
        let mut fire = false;
        if b.next_fire_tick() <= state.tick {
            if let Some(tid) = fire_at[id as usize] {
                if let Some(en) = them.iter().find(|e| e.id == tid) {
                    let tp = en.pos + en.vel;
                    let hit = hull_hit(nxt, tp, &them, range, br).unwrap_or(tid);
                    if hit == tid
                        && nxt.dist(&tp) <= range
                        && in_face(nang, want, arc)
                        && has_line_of_sight(conf, nxt, tp)
                    {
                        fire = true;
                    }
                }
            }
        }
        slot.move_action = move_bot(dir);
        slot.turn_action = turn_towards(look);
        slot.special_action = SpecialAction::Battle { fire };
    }

    let can_rush = !end
        && state.fabricator_me.tokens >= conf.fabricator.rush_cost
        && me.len() < BOTS_MAX as usize;
    let near_lock =
        state.tick + conf.fabricator.interval + 8 >= conf.max_ticks - conf.endgame_ticks;
    action.fabricator_next = if !end && n_ext < 3 {
        BotClass::Extractor
    } else if n_heal < 2 && n_bat >= 3 {
        BotClass::Healer
    } else if n_heal < 3 && n_bat >= 7 {
        BotClass::Healer
    } else {
        BotClass::Battle
    };
    if them_cap > 0 && us_cap == 0 {
        action.fabricator_next = BotClass::Battle;
    }
    if n_heal == 0 && n_bat >= 3 && !(them_cap > 0 && us_cap == 0) {
        action.fabricator_next = BotClass::Healer;
    }
    action.rush_order = can_rush
        && ((them_cap > 0 && us_cap == 0)
            || (n_heal == 0 && n_bat >= 3)
            || (them.len() >= me.len() + 3 && n_ext >= 3)
            || (near_lock && me.len() < 18));
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peel_when_next_shot_is_lethal() {
        assert!(needs_retreat(3.0, 3.0, 10.0));
        assert!(needs_retreat(4.0, 3.0, 10.0));
        assert!(!needs_retreat(9.0, 3.0, 10.0));
    }
}
