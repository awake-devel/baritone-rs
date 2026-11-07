// native_movement_input: only the jump, for the pillar trial. Runs with a screen open too (measured by input-probe).
fn baritone_bp_movement(jump: &mut bool) {
    use crate::game::baritone_bp as m;
    if m::intent().jump {
        *jump = true;
    }
}
