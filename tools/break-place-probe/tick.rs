// Runs in the window's Game only, never the integrated server's. Drives the scenario of runtime.rs from native_tick, which runs
// after the tick's input step: what is decided here acts in the next tick.
fn baritone_bp_tick(game: &mut crate::game::Game<'_>) {
    use crate::game::baritone_bp as m;
    use crate::game::{Screen, chat::ChatExit};
    if !game.native_window() || !m::enabled() {
        return;
    }
    // The input step puts the chat's screen back itself; this is only a safety net.
    if m::take_swapped() && game.screen == Screen::Playing {
        game.screen = Screen::Chat;
    }
    let tool = if m::wants_tool() {
        let pos = m::target_pos();
        let state = game.world.block(pos[0], pos[1], pos[2]);
        let facts = game.res.facts().get(state).clone();
        let held_name = game.held().map(|s| s.item.to_string()).unwrap_or_else(|| "none".to_string());
        let speed = game.items().mining_speed(game.held(), &facts.name);
        let correct = game.items().correct_for_drops(game.held(), &facts.name);
        let bbs = game.block_break_speed();
        let progress = game.destroy_progress(pos);
        let predicted = if progress > 0.0 { (1.0 / progress).ceil() as i64 } else { -1 };
        Some(format!(
            "held={held_name} block={} hardness={} requires_tool={} mining_speed={speed} correct_for_drops={correct} block_break_speed={bbs} destroy_progress={progress:.6} predicted_ticks={predicted}",
            facts.name, facts.hardness, facts.requires_tool
        ))
    } else {
        None
    };
    let name_at = |game: &crate::game::Game<'_>, p: [i32; 3]| -> String {
        game.res.facts().get(game.world.block(p[0], p[1], p[2])).name.to_string()
    };
    let obs = game.player.as_ref().map(|p| (p.position.y, p.ground.on_ground)).map(|(feet_y, on_ground)| m::Obs {
        feet_y,
        on_ground,
        chat: game.screen == Screen::Chat,
        selected: game.selected,
        held: game.held().map(|s| s.item.to_string()).unwrap_or_else(|| "none".to_string()),
        target: name_at(game, m::TARGET_POS),
        front: name_at(game, m::FRONT_POS),
        under: name_at(game, m::UNDER_POS),
        tool,
    });
    let out = m::step(obs);
    for line in out.console {
        // The window's single-player server answers a command as its owner (the same call horizon-ctl's `command` makes).
        let answer = game
            .integrated
            .as_ref()
            .and_then(|server| {
                let sent = line.clone();
                server.call(move |g| g.owner_console_command(&sent))
            })
            .map(|lines| lines.join(" | "))
            .unwrap_or_else(|| "NO SERVER ANSWER".to_string());
        m::log_console(&line, &answer);
    }
    if let Some(open) = out.chat {
        if open {
            if game.screen != Screen::Chat {
                game.open_chat_method("");
            }
        } else if game.screen == Screen::Chat {
            game.close_chat(ChatExit::Intentional);
        }
    }
    if let Some(slot) = out.selected {
        game.selected = slot;
    }
    if let Some((yaw, pitch)) = out.aim {
        game.yaw = yaw;
        game.pitch = pitch;
    }
    if !m::intent().active {
        game.left = false;
        game.right = false;
    }
    if let Some(done) = out.finished {
        let written = m::write_all();
        game.native_chat(&format!("baritone-bp-probe finished ok={done} written={written}"));
    }
}
