use super::*;
use horizon_core::ticks::TickPriority;

const AT: BlockPos = BlockPos::new(5, 64, 5);

fn schedule(w: &mut World<'_>, state: BlockStateId) {
    let block = w.b().block(state);
    w.ticks
        .schedule_block(w.level.time, AT, block, 1, TickPriority::Normal);
}

#[test]
fn farmland_and_path_ask_before_natural_dirt_conversion_and_entity_push() {
    for (block, covered) in [("farmland", false), ("farmland", true), ("dirt_path", true)] {
        let mut w = world!();
        let dirt = w.id("dirt", &[]);
        let from = w.id(block, &[]);
        w.put(AT, from);
        if covered {
            w.put(AT.offset(0, 1, 0), w.id("stone", &[]));
        }
        let tick = |w: &mut World<'_>| {
            if covered {
                schedule(w, from);
                w.tick();
            } else {
                w.b().random_tick(&mut w.level, &mut w.ticks, AT);
            }
        };

        w.level.guarded.insert(AT);
        tick(&mut w);
        assert_eq!(w.get(AT), from, "{block}, covered={covered}");
        assert_eq!(w.level.fade_checks.take(), [(AT, from, dirt)]);
        assert!(w.level.events.is_empty(), "no push or vibration on a veto");

        w.level.guarded.remove(&AT);
        tick(&mut w);
        assert_eq!(w.get(AT), dirt, "{block}, covered={covered}");
        assert_eq!(w.level.fade_checks.take(), [(AT, from, dirt)]);
        assert!(w.level.events.iter().any(|event| matches!(
            event,
            LevelEvent::BlockEvent { pos, kind: "push_entities_up", .. } if *pos == AT
        )));
        assert!(w.level.events.iter().any(|event| matches!(
            event,
            LevelEvent::GameEvent { pos, event: "minecraft:block_change", .. } if *pos == AT
        )));
    }
}

#[test]
fn uncovered_dirt_path_keeps_its_state_when_a_scheduled_tick_is_stale() {
    let mut w = world!();
    let path = w.id("dirt_path", &[]);
    let above = AT.offset(0, 1, 0);
    w.put(AT, path);
    w.put(above, w.id("stone", &[]));
    schedule(&mut w, path);
    w.put(above, w.b().air());
    w.tick();
    assert_eq!(w.get(AT), path);
    assert!(w.level.fade_checks.take().is_empty());
    assert!(w.level.events.is_empty());
}

#[test]
fn covered_nylium_asks_before_returning_to_netherrack() {
    for name in ["crimson_nylium", "warped_nylium"] {
        let mut w = world!();
        let nylium = w.id(name, &[]);
        let netherrack = w.id("netherrack", &[]);
        w.put(AT, nylium);
        w.put(AT.offset(0, 1, 0), w.id("stone", &[]));
        for allowed in [false, true] {
            if allowed {
                w.level.guarded.remove(&AT);
            } else {
                w.level.guarded.insert(AT);
            }
            w.b().random_tick(&mut w.level, &mut w.ticks, AT);
            assert_eq!(w.get(AT), if allowed { netherrack } else { nylium });
            assert_eq!(w.level.fade_checks.take(), [(AT, nylium, netherrack)]);
        }
    }
}

#[test]
fn frosted_ice_keeps_its_age_on_a_veto_and_retries_on_its_next_tick() {
    let mut w = world!();
    let from = w.id("frosted_ice", &[("age", "1")]);
    let older = w.id("frosted_ice", &[("age", "2")]);
    w.put(AT, from);
    w.level.guarded.insert(AT);
    schedule(&mut w, from);
    w.tick();
    assert_eq!(w.get(AT), from);
    assert_eq!(w.level.fade_checks.take(), [(AT, from, older)]);
    assert!(w.level.events.is_empty());

    w.level.guarded.remove(&AT);
    for _ in 0..40 {
        w.tick();
        if w.get(AT) != from {
            break;
        }
    }
    assert!((21..=41).contains(&w.level.time));
    assert_eq!(w.level.fade_checks.take(), [(AT, from, older)]);
    assert_eq!(w.get(AT), older);
}

#[test]
fn healthy_frogspawn_asks_before_hatching_but_lost_support_removes_it() {
    let mut w = world!();
    let frogspawn = w.id("frogspawn", &[]);
    let water = w.id("water", &[("level", "0")]);
    let air = w.b().air();
    let below = AT.offset(0, -1, 0);
    w.put(below, water);
    w.put(AT, frogspawn);
    w.level.guarded.insert(AT);
    schedule(&mut w, frogspawn);
    w.tick();
    assert_eq!(w.get(AT), frogspawn);
    assert_eq!(w.level.fade_checks.take(), [(AT, frogspawn, air)]);
    assert!(w.level.events.is_empty());
    assert!(!w.ticks.has_block_tick(AT, w.b().block(frogspawn)));

    w.level.guarded.remove(&AT);
    schedule(&mut w, frogspawn);
    w.tick();
    assert_eq!(w.get(AT), air);
    assert_eq!(w.level.fade_checks.take(), [(AT, frogspawn, air)]);
    assert!(w.level.events.iter().any(|event| matches!(
        event,
        LevelEvent::BlockEvent { pos, kind: "hatch", .. } if *pos == AT
    )));

    w.put(below, air);
    w.put(AT, frogspawn);
    w.level.events.clear();
    w.level.guarded.insert(AT);
    schedule(&mut w, frogspawn);
    w.tick();
    assert_eq!(w.get(AT), air);
    assert!(w.level.fade_checks.take().is_empty());
    assert!(
        !w.level
            .events
            .iter()
            .any(|event| matches!(event, LevelEvent::BlockEvent { kind: "hatch", .. }))
    );
}

#[test]
fn turtle_eggs_ask_at_final_hatch_and_try_again_on_a_later_random_tick() {
    let mut w = world!();
    let cracked = w.id("turtle_egg", &[("hatch", "1"), ("eggs", "2")]);
    let mature = w.id("turtle_egg", &[("hatch", "2"), ("eggs", "2")]);
    let air = w.b().air();
    w.put(AT.offset(0, -1, 0), w.id("sand", &[]));
    w.put(AT, cracked);
    w.level.day_time = 21500;
    w.level.guarded.insert(AT);
    w.b().random_tick(&mut w.level, &mut w.ticks, AT);
    assert_eq!(w.get(AT), mature);
    assert!(w.level.fade_checks.take().is_empty());

    w.level.events.clear();
    w.b().random_tick(&mut w.level, &mut w.ticks, AT);
    assert_eq!(w.get(AT), mature);
    assert_eq!(w.level.fade_checks.take(), [(AT, mature, air)]);
    assert!(w.level.events.is_empty());

    w.level.guarded.remove(&AT);
    w.b().random_tick(&mut w.level, &mut w.ticks, AT);
    assert_eq!(w.get(AT), air);
    assert_eq!(w.level.fade_checks.take(), [(AT, mature, air)]);
    assert!(w.level.events.iter().any(|event| matches!(
        event,
        LevelEvent::BlockEvent { pos, state, kind: "hatch_turtles" }
            if *pos == AT && *state == mature
    )));
}

#[test]
fn sniffer_eggs_retry_at_the_current_hatch_speed_after_a_veto() {
    for (before, support, expected_delay) in
        [("stone", "moss_block", 4000), ("moss_block", "stone", 8000)]
    {
        let mut w = world!();
        let egg = w.id("sniffer_egg", &[("hatch", "2")]);
        let air = w.b().air();
        let below = AT.offset(0, -1, 0);
        w.put(AT, egg);
        w.put(below, w.id(before, &[]));
        schedule(&mut w, egg);
        w.put(below, w.id(support, &[]));

        let jitter = w.level.random.clone().next_int_bound(300).unwrap();
        w.level.guarded.insert(AT);
        w.tick();
        assert_eq!(w.get(AT), egg);
        assert_eq!(w.level.fade_checks.take(), [(AT, egg, air)]);
        assert!(w.level.events.is_empty());

        w.level.guarded.remove(&AT);
        for _ in 1..(expected_delay + jitter) {
            w.tick();
        }
        assert_eq!(w.get(AT), egg);
        assert!(w.level.fade_checks.take().is_empty());
        assert!(w.level.events.is_empty());

        w.tick();
        assert_eq!(w.get(AT), air);
        assert_eq!(w.level.fade_checks.take(), [(AT, egg, air)]);
        assert!(w.level.events.iter().any(|event| matches!(
            event,
            LevelEvent::BlockEvent { pos, state, kind: "hatch_sniffer" }
                if *pos == AT && *state == egg
        )));
    }
}
