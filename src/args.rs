//! Command-line arguments, parsed by hand to keep dependencies light.

use std::time::Duration;

pub const USAGE: &str = "Usage: appmem [--once] [--interval <seconds>]

  --once                 print the per-app table once and exit
  --interval <seconds>   refresh interval, at least 0.5 (default 2)
  -h, --help             show this help
  -V, --version          show the version";

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub once: bool,
    pub interval: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Run(Args),
    Help,
    Version,
}

pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut parsed = Args {
        once: false,
        interval: Duration::from_secs(2),
    };
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--once" => parsed.once = true,
            "--interval" => {
                let value = args.next().ok_or("--interval needs a value in seconds")?;
                let secs: f64 = value.parse().map_err(|_| format!("invalid interval: {value}"))?;
                if !(secs >= 0.5 && secs.is_finite()) {
                    return Err("interval must be at least 0.5 seconds".to_string());
                }
                parsed.interval = Duration::from_secs_f64(secs);
            }
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Parsed::Run(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parse_defaults() {
        assert_eq!(
            parse(&[]),
            Ok(Parsed::Run(Args {
                once: false,
                interval: Duration::from_secs(2)
            }))
        );
    }

    #[test]
    fn parse_interval() {
        assert_eq!(
            parse(&["--interval", "0.5", "--once"]),
            Ok(Parsed::Run(Args {
                once: true,
                interval: Duration::from_millis(500)
            }))
        );
    }

    #[test]
    fn rejects_tiny_interval() {
        assert!(parse(&["--interval", "0.1"]).is_err());
        assert!(parse(&["--interval", "abc"]).is_err());
        assert!(parse(&["--interval"]).is_err());
    }

    #[test]
    fn rejects_unknown_flag() {
        assert_eq!(parse(&["--nope"]), Err("unknown argument: --nope".to_string()));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse(&["-h"]), Ok(Parsed::Help));
        assert_eq!(parse(&["--version"]), Ok(Parsed::Version));
    }
}
