// SPDX-License-Identifier: MPL-2.0
use crate::{engine::Action, ui::UiAction};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Pointer {
    pub pressed: Option<usize>,
    release_action: Option<UiAction>,
    repeat: Option<Instant>,
}
impl Pointer {
    pub fn down(
        &mut self,
        id: Option<usize>,
        action: Option<UiAction>,
        vr: bool,
        now: Instant,
    ) -> Option<UiAction> {
        self.pressed = id;
        self.repeat = if matches!(action, Some(UiAction::Input(Action::Backspace))) {
            Some(now + Duration::from_millis(450))
        } else {
            None
        };
        if vr {
            self.release_action = None;
            action
        } else {
            self.release_action = action;
            None
        }
    }
    pub fn up(&mut self, hit: Option<usize>) -> Option<UiAction> {
        let action = if self.pressed.is_some() && hit == self.pressed {
            self.release_action.take()
        } else {
            None
        };
        self.cancel();
        action
    }
    pub fn repeat(&mut self, hover: Option<usize>, now: Instant) -> bool {
        if self.repeat.is_some_and(|next| now >= next)
            && self.pressed.is_some()
            && hover == self.pressed
        {
            self.repeat = Some(now + Duration::from_millis(75));
            self.release_action = None;
            true
        } else {
            false
        }
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vr_press_is_immediate_and_release_never_types_twice() {
        let mut p = Pointer::default();
        let action = p.down(
            Some(36),
            Some(UiAction::Input(Action::Key('n'))),
            true,
            Instant::now(),
        );
        assert!(matches!(action, Some(UiAction::Input(Action::Key('n')))));
        assert!(p.up(Some(36)).is_none());
        p.down(
            Some(36),
            Some(UiAction::Input(Action::Key('n'))),
            true,
            Instant::now(),
        );
        assert!(p.up(Some(27)).is_none()); // trigger release drift must not type J
    }
    #[test]
    fn desktop_release_cancel_and_repeat() {
        let mut p = Pointer::default();
        let now = Instant::now();
        assert!(
            p.down(
                Some(36),
                Some(UiAction::Input(Action::Key('n'))),
                false,
                now
            )
            .is_none()
        );
        assert!(matches!(
            p.up(Some(36)),
            Some(UiAction::Input(Action::Key('n')))
        ));
        p.down(
            Some(36),
            Some(UiAction::Input(Action::Key('n'))),
            false,
            now,
        );
        p.cancel();
        assert!(p.up(Some(36)).is_none());
        assert!(matches!(
            p.down(Some(8), Some(UiAction::Input(Action::Backspace)), true, now),
            Some(UiAction::Input(Action::Backspace))
        ));
        assert!(!p.repeat(Some(8), now + Duration::from_millis(449)));
        assert!(p.repeat(Some(8), now + Duration::from_millis(450)));
        assert!(!p.repeat(Some(8), now + Duration::from_millis(451)));
        assert!(!p.repeat(Some(9), now + Duration::from_millis(525)));
        assert!(p.up(Some(8)).is_none());
        assert!(!p.repeat(Some(8), now + Duration::from_secs(1)));
    }
}
