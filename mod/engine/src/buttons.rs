// baritone-rs engine: the mouse buttons the executor holds. Written for this project. LGPL-3.0-or-later, see lib.rs.
//
// The executor asks for the left button (`Intent::attack`) and the right button (one press, `Intent::use_item`) every tick; the game reads its buttons
// in its input step, which is not in step with the ticks. This keeps what the next step must do: hold a button, or let go of one the mod held and no
// longer wants, exactly once. After that the player's own mouse is theirs again.

/// The mouse buttons the mod holds. `drive_*` says the mod owns that button in the next input step; `left` and `right` are the values to put there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub drive_left: bool,
    pub left: bool,
    pub drive_right: bool,
    pub right: bool,
    /// What the mod asked for last time, to let go once when a button is no longer wanted.
    was_left: bool,
    was_right: bool,
}

impl Buttons {
    pub const fn new() -> Buttons {
        Buttons { drive_left: false, left: false, drive_right: false, right: false, was_left: false, was_right: false }
    }

    /// What the next input step holds. A let-go stays pending until a step has delivered it ([`Buttons::step_done`]), so a second call before that
    /// step (a `/b stop` twice, a stop and a release) cannot lose it.
    pub fn set(&mut self, left: bool, right: bool) {
        self.drive_left = left || self.was_left || self.drive_left;
        self.left = left;
        self.drive_right = right || self.was_right || self.drive_right;
        self.right = right;
        self.was_left = left;
        self.was_right = right;
    }

    /// Whether the mod owns a button in the next input step.
    pub fn driving(&self) -> bool {
        self.drive_left || self.drive_right
    }

    /// The game's input step ran with these values: a button that is no longer wanted has been let go of, and is the player's again.
    pub fn step_done(&mut self) {
        if !self.left {
            self.drive_left = false;
        }
        if !self.right {
            self.drive_right = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Buttons;

    #[test]
    fn idle_buttons_are_never_driven() {
        let mut b = Buttons::new();
        for _ in 0..5 {
            b.set(false, false);
            assert!(!b.driving());
        }
    }

    #[test]
    fn a_held_button_is_let_go_of_exactly_once() {
        let mut b = Buttons::new();
        b.set(true, false);
        assert!(b.driving() && b.left);
        b.step_done();
        assert!(b.driving(), "still held");
        b.set(false, false);
        assert!(b.driving() && !b.left, "the let-go is pending");
        b.step_done();
        assert!(!b.driving(), "delivered once, then the player's mouse");
        b.set(false, false);
        assert!(!b.driving());
    }

    #[test]
    fn a_second_stop_before_the_step_does_not_lose_the_let_go() {
        let mut b = Buttons::new();
        b.set(true, true);
        b.step_done();
        b.set(false, false);
        b.set(false, false);
        assert!(b.drive_left && b.drive_right && !b.left && !b.right);
        b.step_done();
        assert!(!b.driving());
    }

    #[test]
    fn a_step_that_did_not_run_leaves_the_let_go_pending() {
        let mut b = Buttons::new();
        b.set(true, false);
        b.set(false, false);
        // no step_done: an inventory screen was open
        b.set(false, false);
        assert!(b.drive_left);
        b.step_done();
        assert!(!b.drive_left);
    }

    #[test]
    fn each_button_is_released_on_its_own() {
        let mut b = Buttons::new();
        b.set(true, true);
        b.step_done();
        b.set(true, false);
        assert!(b.drive_right && !b.right);
        b.step_done();
        assert!(b.drive_left && !b.drive_right, "the right button is the player's again while the left is held");
    }
}
