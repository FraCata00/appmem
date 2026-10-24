use std::process::ExitCode;

use appmem::args::{Parsed, USAGE, parse_args};
use appmem::collector::{Collector, MacCollector, PlistBundles};
use appmem::grouping::build_groups;
use appmem::report::render_report;

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Version) => {
            println!("appmem {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("appmem: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let mut collector = MacCollector::new();
    if args.once {
        let snap = match collector.snapshot() {
            Ok(snap) => snap,
            Err(err) => {
                eprintln!("appmem: {err}");
                return ExitCode::FAILURE;
            }
        };
        let groups = build_groups(&snap, &mut PlistBundles::default());
        let width = terminal_width().unwrap_or(80);
        print!("{}", render_report(&snap.stats, &groups, width));
        return ExitCode::SUCCESS;
    }

    match appmem::ui::run::run(&mut collector, args.interval) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("appmem: {err}");
            ExitCode::FAILURE
        }
    }
}

fn terminal_width() -> Option<usize> {
    ratatui::crossterm::terminal::size().ok().map(|(w, _)| w as usize)
}
