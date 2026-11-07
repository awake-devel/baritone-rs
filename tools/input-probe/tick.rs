// Runs in the window's Game only, never the integrated server's. Drives the scenario of runtime.rs.
fn baritone_probe_tick(game: &mut crate::game::Game<'_>) {
    use crate::game::baritone_probe as m;
    use crate::game::{Screen, chat::ChatExit};
    if !game.native_window() || !m::enabled() {
        return;
    }
    let screen_kind = match game.screen {
        Screen::Playing => m::ScreenKind::Playing,
        Screen::Chat => m::ScreenKind::Chat,
        Screen::Inventory => m::ScreenKind::Inventory,
        _ => m::ScreenKind::Other,
    };
    let obs = game.player.as_ref().map(|p| m::Obs {
        pos: [p.position.x, p.position.y, p.position.z],
        vel: [p.velocity.x, p.velocity.y, p.velocity.z],
        yaw: game.yaw,
        on_ground: p.ground.on_ground,
        sprinting: p.sprinting,
        sprint_modifier: p.has_sprint_modifier(),
        screen: screen_kind,
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
    if let Some(kind) = out.screen {
        match kind {
            m::ScreenKind::Chat => game.open_chat_method(""),
            m::ScreenKind::Inventory => {
                game.screen = Screen::Inventory;
                game.keys.clear();
                game.capture(false);
            }
            _ => match game.screen {
                Screen::Chat => game.close_chat(ChatExit::Intentional),
                Screen::Inventory => {
                    game.screen = Screen::Playing;
                    game.capture(true);
                }
                _ => {}
            },
        }
    }
    if let Some(yaw) = out.yaw {
        game.yaw = yaw;
    }
    if let Some(on) = out.set_sprinting
        && let Some(p) = game.player.as_mut()
    {
        let _ = p.set_sprinting(on);
    }
    if let Some(done) = out.finished {
        let written = m::write_all();
        game.native_chat(&format!("baritone-probe finished ok={done} written={written}"));
    }
}
