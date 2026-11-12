// SPDX-License-Identifier: GPL-3.0-or-later
mod app;
mod ui_bridge;
mod window_state;

/// The AppImage puts its share/ folder, with the vendored GearLeverCosmic icon
/// theme, on XDG_DATA_DIRS. A build run from the source tree (cargo run, test
/// windows) has no such entry and would show no symbolic icons, so it adds the
/// repository's resources/ folder instead.
fn add_source_tree_icon_theme() {
    if gearlever_cosmic::appimage_self().is_some() || gearlever_cosmic::host::sandboxed() {
        return;
    }
    let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    if !resources
        .join("icons/GearLeverCosmic/index.theme")
        .is_file()
    {
        return;
    }
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    if dirs
        .split(':')
        .any(|d| std::path::Path::new(d) == resources)
    {
        return;
    }
    // SAFETY: the first thing main does, before any other thread exists.
    unsafe { std::env::set_var("XDG_DATA_DIRS", format!("{dirs}:{}", resources.display())) };
}

fn main() -> cosmic::iced::Result {
    add_source_tree_icon_theme();
    let paths = gearlever_cosmic::paths::Paths::discover().expect("HOME must be set");
    let settings = gearlever_cosmic::settings::Settings::load(&paths).unwrap_or_default();
    let level = if settings.debug_logs { "debug" } else { "warn" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
    let mut flags = app::Flags::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                let own = gearlever_cosmic::appimage_self()
                    .map(|p| gearlever_cosmic::shell_quote(&p.to_string_lossy()))
                    .unwrap_or_else(|| "gearlever-cosmic".into());
                let cli = gearlever_cosmic::cli_command();
                println!(
                    "Gear Lever COSMIC\n\n{own} [FILE.AppImage...]\n  --preview PAGE      Open apps, app, updates, add, drop, settings, about or remove (add -narrow for the minimum width)\n  --capture FILE.png  Save the window's own render and exit\n  --help              Show this help\n\nAgents and scripts: the command below manages AppImages without a window\nand controls this window (ui inspect / ui activate). Start with:\n  {cli} guide"
                );
                return Ok(());
            }
            // Relaunch after an update: start once the old instance has exited,
            // since a second instance would only activate it.
            "--wait-for-pid" => {
                let pid: u32 = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .expect("--wait-for-pid requires a process id");
                let start = std::time::Instant::now();
                while std::path::Path::new(&format!("/proc/{pid}")).exists()
                    && start.elapsed() < std::time::Duration::from_secs(30)
                {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
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
