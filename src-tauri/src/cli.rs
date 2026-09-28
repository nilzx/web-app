//! Command-line arguments.

use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Cli {
    pub config: Option<PathBuf>,
    pub apps_dir: Option<PathBuf>,
    /// Open this app directly.
    pub app: Option<String>,
    /// Always show the launcher, even with a single app.
    pub launcher: bool,
    pub help: bool,
    pub version: bool,
}

pub const HELP: &str = "\
WebDock - desktop shell for local web apps

USAGE:
    webdock [OPTIONS]

OPTIONS:
    -c, --config <FILE>     Use this config file (default: webdock.toml next to the
                            executable, else the system config folder)
    -d, --apps-dir <DIR>    Override the apps folder from the config
    -a, --app <ID>          Open the app with this id directly
    -l, --launcher          Always show the launcher
    -h, --help              Print this help
    -V, --version           Print the version
";

impl Cli {
    /// Parses arguments (without the program name). Unknown arguments are
    /// ignored so OS-specific extras (e.g. macOS `-psn_…`) don't break startup.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Self {
        let mut cli = Cli::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                _ => (arg.clone(), None),
            };
            let mut value = || inline.clone().or_else(|| it.next());
            match flag.as_str() {
                "-c" | "--config" => cli.config = value().map(PathBuf::from),
                "-d" | "--apps-dir" => cli.apps_dir = value().map(PathBuf::from),
                "-a" | "--app" => cli.app = value().filter(|v| !v.is_empty()),
                "-l" | "--launcher" => cli.launcher = true,
                "-h" | "--help" => cli.help = true,
                "-V" | "--version" => cli.version = true,
                _ => {}
            }
        }
        cli
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[&str]) -> Cli {
        Cli::parse(s.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses() {
        let c = parse(&["--app", "notes", "--config=/x/y.toml", "-l", "-psn_0_1"]);
        assert_eq!(c.app.as_deref(), Some("notes"));
        assert_eq!(c.config, Some(PathBuf::from("/x/y.toml")));
        assert!(c.launcher);
        assert_eq!(parse(&["-d", "apps"]).apps_dir, Some(PathBuf::from("apps")));
        assert_eq!(parse(&[]), Cli::default());
    }
}
