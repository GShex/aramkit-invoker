use std::ffi::OsStr;
use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use zeroize::Zeroize;

/// Local credentials exposed by the League client on the loopback interface.
pub struct LcuCredentials {
    pub port: u16,
    pub token: String,
}

impl std::fmt::Debug for LcuCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LcuCredentials")
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Drop for LcuCredentials {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

impl LcuCredentials {
    pub fn base_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }
}

/// Finds `--app-port` and `--remoting-auth-token` in a League process command line.
pub fn parse_lcu_args<I, S>(args: I) -> Option<LcuCredentials>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect();

    let mut port = None;
    let mut token = None;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].trim_matches('"');

        if let Some(value) = arg.strip_prefix("--app-port=") {
            port = parse_port(value);
        } else if arg == "--app-port" {
            if let Some(value) = args.get(index + 1) {
                port = parse_port(value.trim_matches('"'));
                index += 1;
            }
        } else if let Some(value) = arg.strip_prefix("--remoting-auth-token=") {
            token = clean_token(value);
        } else if arg == "--remoting-auth-token" {
            if let Some(value) = args.get(index + 1) {
                token = clean_token(value);
                index += 1;
            }
        }

        index += 1;
    }

    // Do not leave a copy of the token in the temporary argument vector.
    for arg in &mut args {
        arg.zeroize();
    }

    match (port, token) {
        (Some(port), Some(token)) if port > 0 && !token.is_empty() => {
            Some(LcuCredentials { port, token })
        }
        _ => None,
    }
}

/// Parses the client's `lockfile` (`name:pid:port:password:protocol`).
pub fn parse_lockfile(contents: &str) -> Option<LcuCredentials> {
    let parts: Vec<&str> = contents.trim().splitn(5, ':').collect();
    if parts.len() < 5 {
        return None;
    }

    let port = parts[2].parse::<u16>().ok().filter(|port| *port > 0)?;
    let token = parts[3].trim();
    if token.is_empty() {
        return None;
    }

    Some(LcuCredentials {
        port,
        token: token.to_owned(),
    })
}

/// Scans running processes for the League client and extracts its credentials.
///
/// The client's command line is the primary source. If it is unavailable, the
/// `lockfile` next to the executable is used as a fallback.
pub fn discover_credentials() -> Option<LcuCredentials> {
    let mut system = System::new().ok()?;
    let refresh = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_exe(UpdateKind::OnlyIfNotSet)
        .without_tasks();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

    // Deterministic order: prefer the most recently started client when more
    // than one League client is running.
    let mut processes: Vec<_> = system
        .processes()
        .iter()
        .filter(|(_, process)| is_league_client_process(process.name()))
        .collect();
    processes.sort_by_key(|&(pid, _process)| std::cmp::Reverse(pid.as_u32()));

    for (_, process) in &processes {
        let mut args: Vec<String> = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        let mut credentials = parse_lcu_args(args.iter());

        // Some Windows configurations return the command line as one string.
        if credentials.is_none() && args.len() == 1 {
            credentials = parse_lcu_args(args[0].split_whitespace());
        }

        for arg in &mut args {
            arg.zeroize();
        }

        if credentials.is_some() {
            return credentials;
        }
    }

    // Only read the lockfile when no command line yielded credentials.
    for (_, process) in processes {
        if let Some(executable) = process.exe() {
            if let Some(credentials) = read_lockfile_near(executable) {
                return Some(credentials);
            }
        }
    }

    None
}

fn is_league_client_process(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    let file_name = name.rsplit(['\\', '/']).next().unwrap_or(name.as_str());
    file_name == "leagueclientux.exe" || file_name == "leagueclientux"
}

fn read_lockfile_near(executable: &Path) -> Option<LcuCredentials> {
    let directory = executable.parent()?;
    for candidate in [
        directory.join("lockfile"),
        directory.join("..").join("lockfile"),
    ] {
        if let Ok(mut contents) = std::fs::read_to_string(&candidate) {
            let credentials = parse_lockfile(&contents);
            contents.zeroize();
            if credentials.is_some() {
                return credentials;
            }
        }
    }
    None
}

fn parse_port(value: &str) -> Option<u16> {
    value
        .trim()
        .trim_matches('"')
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
}

fn clean_token(value: &str) -> Option<String> {
    let token = value.trim().trim_matches('"');
    (!token.is_empty()).then(|| token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_league_client_arguments() {
        let args = [
            r"C:\Riot Games\League of Legends\LeagueClientUx.exe",
            "--app-port=54321",
            "--remoting-auth-token=abc123",
            "--locale=zh_CN",
        ];

        let credentials = parse_lcu_args(args).expect("credentials");
        assert_eq!(credentials.port, 54321);
        assert_eq!(credentials.token, "abc123");
    }

    #[test]
    fn parses_space_separated_arguments() {
        let args = ["--app-port", "12345", "--remoting-auth-token", "token"];
        let credentials = parse_lcu_args(args).expect("credentials");

        assert_eq!(credentials.port, 12345);
        assert_eq!(credentials.token, "token");
    }

    #[test]
    fn rejects_missing_or_invalid_values() {
        assert!(parse_lcu_args(["--app-port=0", "--remoting-auth-token=x"]).is_none());
        assert!(parse_lcu_args(["--app-port=1234"]).is_none());
        assert!(parse_lcu_args(["--remoting-auth-token=x"]).is_none());
        assert!(parse_lcu_args(["--app-port=abc", "--remoting-auth-token=x"]).is_none());
    }

    #[test]
    fn parses_lockfile() {
        let credentials =
            parse_lockfile("LeagueClient:1234:54321:secret-token:https").expect("credentials");

        assert_eq!(credentials.port, 54321);
        assert_eq!(credentials.token, "secret-token");
    }

    #[test]
    fn rejects_bad_lockfile() {
        assert!(parse_lockfile("too:short").is_none());
        assert!(parse_lockfile("LeagueClient:1234:0:token:https").is_none());
        assert!(parse_lockfile("LeagueClient:1234:54321::https").is_none());
    }

    #[test]
    fn recognizes_client_process_names() {
        assert!(is_league_client_process(OsStr::new("LeagueClientUx.exe")));
        assert!(is_league_client_process(OsStr::new(
            r"C:\Riot Games\LeagueClientUx.exe"
        )));
        assert!(!is_league_client_process(OsStr::new("LeagueClient.exe")));
        assert!(!is_league_client_process(OsStr::new(
            "LeagueClientUxRender.exe"
        )));
    }

    #[test]
    fn debug_output_hides_token() {
        let credentials = LcuCredentials {
            port: 1234,
            token: "super-secret".to_owned(),
        };

        let debug = format!("{credentials:?}");
        assert!(debug.contains("1234"));
        assert!(!debug.contains("super-secret"));
    }
}
