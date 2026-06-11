#[derive(Debug, PartialEq, Eq)]
pub enum RunMode {
    ConsoleWorker,
    SettingsUi,
    BackgroundApp,
}

impl Default for RunMode {
    fn default() -> Self {
        Self::BackgroundApp
    }
}

#[derive(Default)]
pub struct Args {
    pub debug: bool,
    pub verbose: bool,
    pub combine_enabled: bool,
    pub reopen_ui: bool,
    pub mode: RunMode,
}

pub fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    let mut args = Args::default();

    args.debug = raw.iter().any(|a| a == "--debug");
    args.verbose = raw.iter().any(|a| a == "-v" || a == "--verbose");
    args.combine_enabled = raw.iter().any(|a| a == "--combine-mode");
    args.reopen_ui = raw.iter().any(|a| a == "--reopen-ui");

    if raw.iter().any(|a| a == "--console-worker") {
        args.mode = RunMode::ConsoleWorker;
    } else if raw.iter().any(|a| a == "--settings-ui") {
        args.mode = RunMode::SettingsUi;
    } else {
        args.mode = RunMode::BackgroundApp;
    }

    args
}

pub fn print_help(args: &Args) {
    let mut info = String::from(
        "\nWinGlide started:\
        \n\tAlt+[  : cycle left\
        \n\tAlt+]  : cycle right\
        \n\tRight-click tray icon : menu\
        \n\
        \n\t-v/--verbose: enable debug logging\
        \n\t--combine-mode: enable combine mode\
        \n\t--debug: attach console for debugging",
    );

    if args.verbose {
        info.push_str("\nVerbose logging enabled");
    }

    if args.combine_enabled {
        info.push_str("\nCombine mode enabled");
    }

    if args.debug {
        info.push_str("\nDebug console enabled");
    }

    println!("{}\n", info);
}
