// SPDX-License-Identifier: GPL-3.0-or-later
//! Keep the last normal size separately from maximized/fullscreen dimensions.
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: u32,
    pub height: u32,
    pub mode: Mode,
    /// Width of the app list beside the details pane, in logical pixels.
    pub list_width: u32,
    /// Width of the navigation sidebar, in logical pixels.
    pub nav_width: u32,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1240,
            height: 800,
            mode: Mode::Windowed,
            list_width: 340,
            nav_width: 240,
        }
    }
}
/// Smallest window the layouts were checked at (verification/layout-sweep.sh
/// and drag-sweep.sh): every page and the header title still fit.
pub const MIN_WIDTH: u32 = 380;
pub const MIN_HEIGHT: u32 = 400;
impl WindowState {
    pub fn observe(&mut self, mode: Mode, width: f32, height: f32) {
        self.mode = mode;
        if mode == Mode::Windowed
            && width.is_finite()
            && height.is_finite()
            && width >= MIN_WIDTH as f32
            && height >= MIN_HEIGHT as f32
        {
            self.width = (width.round() as u32).min(16384);
            self.height = (height.round() as u32).min(16384);
        }
    }
    pub fn size(self) -> cosmic::iced::Size {
        cosmic::iced::Size::new(
            self.width.clamp(MIN_WIDTH, 16384) as f32,
            self.height.clamp(MIN_HEIGHT, 16384) as f32,
        )
    }
}
pub fn restore<T: Send + 'static>(
    mode: Mode,
    window: cosmic::iced::window::Id,
) -> cosmic::iced::Task<T> {
    use cosmic::iced::{Task, window as command};
    match mode {
        Mode::Windowed => Task::batch([
            command::set_mode(window, command::Mode::Windowed),
            command::maximize(window, false),
        ]),
        Mode::Maximized => Task::batch([
            command::set_mode(window, command::Mode::Windowed),
            command::maximize(window, true),
        ]),
        Mode::Fullscreen => command::set_mode(window, command::Mode::Fullscreen),
    }
}
fn path() -> std::path::PathBuf {
    gearlever_cosmic::paths::Paths::discover()
        .map(|p| p.config)
        .unwrap_or_default()
        .join("window-state.json")
}
pub fn load() -> Result<WindowState, String> {
    load_from(&path())
}
fn load_from(path: &Path) -> Result<WindowState, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Cannot read window state {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WindowState::default()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn save(state: &WindowState) -> Result<(), String> {
    save_to(&path(), state)
}
fn save_to(path: &Path, state: &WindowState) -> Result<(), String> {
    gearlever_cosmic::settings::write_json(path, state)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reopening_keeps_mode_and_normal_size_across_fullscreen_and_maximize() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("window-state.json");
        let mut state = WindowState::default();
        state.observe(Mode::Windowed, 1100., 700.);
        for mode in [Mode::Maximized, Mode::Fullscreen, Mode::Windowed] {
            state.observe(
                mode,
                if mode == Mode::Windowed { 1100. } else { 1600. },
                if mode == Mode::Windowed { 700. } else { 900. },
            );
            save_to(&path, &state).unwrap();
            let restored = load_from(&path).unwrap();
            assert_eq!(restored.mode, mode);
            assert_eq!((restored.width, restored.height), (1100, 700));
        }
        state.list_width = 377;
        state.nav_width = 301;
        save_to(&path, &state).unwrap();
        let restored = load_from(&path).unwrap();
        assert_eq!((restored.list_width, restored.nav_width), (377, 301));
        // Files from before the dividers existed load with the defaults.
        fs::write(&path, br#"{"width":900,"height":700,"mode":"windowed"}"#).unwrap();
        let old = load_from(&path).unwrap();
        assert_eq!((old.width, old.list_width, old.nav_width), (900, 340, 240));
        save_to(&path, &state).unwrap();
        let before = fs::read_dir(temp.path()).unwrap().count();
        save_to(&path, &state).unwrap();
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), before);
        assert_eq!(
            WindowState {
                width: 0,
                height: u32::MAX,
                ..state
            }
            .size(),
            cosmic::iced::Size::new(MIN_WIDTH as f32, 16384.)
        );
    }
}
