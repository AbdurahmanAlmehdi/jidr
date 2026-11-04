//! Player fullscreen (⌃⌘F, Esc leaves): the device screen letterboxed on
//! the whole window, the title bar, toolbar and widget panel hidden.
//! UI-only view state: it stays in the GUI and is never a protocol command.
//!
//! The native window follows the player: entering asks for native
//! fullscreen (its own Space on macOS). A window that was already
//! fullscreen (the green button) stays fullscreen when the player leaves.

use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;

use crate::app::Ctx;

/// What the player remembers between entering and leaving.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Fullscreen {
    pub on: bool,
    /// The window was natively fullscreen before the player entered.
    native_before: bool,
}

impl Fullscreen {
    /// Applies a request. Returns the native fullscreen state to ask the
    /// window for, or `None` when the window stays as it is.
    pub fn request(&mut self, on: bool, native_now: bool) -> Option<bool> {
        if on == self.on {
            return None;
        }
        self.on = on;
        if on {
            self.native_before = native_now;
            (!native_now).then_some(true)
        } else {
            (!self.native_before).then_some(false)
        }
    }

    /// The window left native fullscreen on its own (the green button,
    /// Mission Control): the player follows. Returns true if it changed.
    pub fn native_left(&mut self) -> bool {
        let was = self.on;
        *self = Self::default();
        was
    }
}

/// Enters or leaves player fullscreen. Only a device tab can be
/// fullscreen; asking from the Library leaves it.
pub fn set(ctx: &Rc<Ctx>, on: bool) {
    let on = on && super::active_id(ctx).is_some();
    let window = ctx.app.window();
    let native = ctx
        .player
        .borrow_mut()
        .fullscreen
        .request(on, window.is_fullscreen());
    ctx.app.set_player_fullscreen(on);
    if let Some(n) = native {
        window.set_fullscreen(n);
    }
}

pub fn toggle(ctx: &Rc<Ctx>) {
    let on = ctx.player.borrow().fullscreen.on;
    set(ctx, !on);
}

/// After a native resize, check (once Slint has synced its window state)
/// whether the window left fullscreen by itself.
pub fn after_resize(ctx: &Rc<Ctx>) {
    if !ctx.player.borrow().fullscreen.on {
        return;
    }
    let c = ctx.clone();
    slint::Timer::single_shot(Duration::from_millis(0), move || {
        if c.app.window().is_fullscreen() {
            return;
        }
        let Ok(mut p) = c.player.try_borrow_mut() else {
            return;
        };
        if p.fullscreen.native_left() {
            drop(p);
            c.app.set_player_fullscreen(false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::Fullscreen;

    #[test]
    fn enter_and_leave_drive_the_native_window() {
        let mut f = Fullscreen::default();
        assert_eq!(f.request(true, false), Some(true));
        assert!(f.on);
        // Asking again changes nothing.
        assert_eq!(f.request(true, true), None);
        assert_eq!(f.request(false, true), Some(false));
        assert!(!f.on);
        assert_eq!(f.request(false, false), None);
    }

    #[test]
    fn a_window_already_fullscreen_stays_fullscreen() {
        let mut f = Fullscreen::default();
        // The green button made the window fullscreen first.
        assert_eq!(f.request(true, true), None);
        assert!(f.on);
        assert_eq!(f.request(false, true), None);
        assert!(!f.on);
    }

    #[test]
    fn leaving_natively_resets_the_player() {
        let mut f = Fullscreen::default();
        assert!(!f.native_left());
        f.request(true, false);
        assert!(f.native_left());
        assert!(!f.on);
        // Entering again asks for native fullscreen again.
        assert_eq!(f.request(true, false), Some(true));
    }
}
