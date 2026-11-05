// Runs in the window's Game only, never the integrated server's. Writes the same fields as Horizon's own agent goto.
fn baritone_walk_tick(game: &mut crate::game::Game<'_>) {
    use crate::game::baritone_walk as m;
    use winit::keyboard::KeyCode;
    if !game.native_window() {
        return;
    }
    let running = m::WALKER.with(|w| w.borrow().running);
    if !running {
        return;
    }
    let Some((p, on_ground)) = game.player.as_ref().map(|p| ([p.position.x, p.position.y, p.position.z], p.ground.on_ground)) else {
        m::WALKER.with(|w| w.borrow_mut().running = false);
        return;
    };
    let yaw = game.yaw;
    let out = m::WALKER.with(|w| w.borrow_mut().step(p, yaw, on_ground));
    if let Some(y) = out.yaw {
        game.yaw = y;
    }
    for (key, on) in [(KeyCode::KeyW, out.forward), (KeyCode::Space, out.jump)] {
        if on {
            game.keys.insert(key);
        } else {
            game.keys.remove(&key);
        }
    }
    // Not through ControlLeft: heal_hold_latches releases the sprint mapping while the keyboard reports Control up.
    if let Some(p) = game.player.as_mut() {
        let _ = p.set_sprinting(out.sprint);
    }
    if let Some(ok) = out.finished {
        let csv = m::WALKER.with(|w| w.borrow().csv());
        let written = std::fs::write(m::log_path(), csv).is_ok();
        game.native_chat(&format!("baritonewalk finished arrived={ok} written={written}"));
    }
}
