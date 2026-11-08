// SPDX-License-Identifier: GPL-3.0-or-later
mod app;
mod ui_bridge;
mod window_state;

fn main() -> cosmic::iced::Result {
    let paths = gearlever_cosmic::paths::Paths::discover().expect("HOME must be set");
    let settings = gearlever_cosmic::settings::Settings::load(&paths).unwrap_or_default();
    let level = if settings.debug_logs { "debug" } else { "warn" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
    let mut flags = app::Flags::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Gear Lever COSMIC\n\ngearlever-cosmic [FILE.AppImage...]\n  --preview PAGE      Open apps, app, updates, add, drop, settings, about or remove (add -narrow for the minimum width)\n  --capture FILE.png  Save the window's own render and exit\n  --help              Show this help\n\nAgents and scripts: gearlever-cosmic-cli manages AppImages without a window and\ncontrols this window (ui inspect / ui activate). Start with:\n  gearlever-cosmic-cli guide"
                );
                return Ok(());
            }
            "--preview" => flags.preview = Some(args.next().expect("--preview requires a page")),
            "--capture" => {
                flags.capture = Some(args.next().expect("--capture requires a PNG path").into())
            }
            v if !v.starts_with('-') => flags
                .open_files
                .push(std::path::absolute(v).expect("Cannot resolve file path")),
            _ => {
                eprintln!("Unknown argument {arg}. See --help.");
                std::process::exit(2);
            }
        }
    }
    let mut state = if flags.capture.is_some() {
        window_state::WindowState::default()
    } else {
        window_state::load().unwrap_or_default()
    };
    if flags.capture.is_some()
        && flags
            .preview
            .as_deref()
            .is_some_and(|p| p.ends_with("-narrow"))
    {
        state.width = window_state::MIN_WIDTH;
    }
    let size = state.size();
    flags.window_state = state;
    flags.activation_action = (!flags.open_files.is_empty()).then(|| "open-files".into());
    let isolated = flags.capture.is_some()
        || std::env::var("GEARLEVER_COSMIC_UI_ISOLATED").is_ok_and(|v| v == "1");
    let cosmic_settings = cosmic::app::Settings::default()
        .theme(app::theme_of(settings.theme))
        .size(size)
        .size_limits(
            cosmic::iced::Limits::NONE
                .min_width(window_state::MIN_WIDTH as f32)
                .min_height(window_state::MIN_HEIGHT as f32),
        )
        .default_icon_theme("GearLeverCosmic");
    if isolated {
        cosmic::app::run::<app::App>(cosmic_settings, flags)
    } else {
        cosmic::app::run_single_instance::<app::App>(cosmic_settings, flags)
    }
}
