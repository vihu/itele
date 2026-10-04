//! Moving between the screens from the sidebar and the keys that mirror
//! it.

use super::Session;
use super::vod::Kind;
use crate::ui::Screen;

impl Session {
    /// Switches between the browse screens from the rail.
    pub(super) fn navigate(&self, item: i32) {
        let place = match item {
            1 => Some(Screen::Live),
            2 => Some(Screen::Guide),
            3 => Some(Screen::Movies),
            4 => Some(Screen::Series),
            _ => None,
        };
        if let Some(screen) = place {
            self.remember_screen(screen);
        }
        match item {
            1 => {
                self.show(Screen::Live);
                self.resume_preview();
                if let Some(app) = self.app.upgrade() {
                    app.invoke_reveal_current();
                }
            }
            2 => {
                self.open_guide();
                self.resume_preview();
            }
            3 => self.open_vod(Kind::Movies),
            4 => self.open_vod(Kind::Series),
            5 => self.open_search(),
            6 => self.open_settings(),
            _ => {}
        }
    }
}
