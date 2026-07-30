use crate::help::USAGE;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Auto,
    Force,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub recursive: bool,
    pub force: bool,
    pub verbose: bool,
    pub progress: ProgressMode,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            recursive: false,
            force: false,
            verbose: false,
            progress: ProgressMode::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub options: Options,
    pub sources: Vec<PathBuf>,
    pub dest: PathBuf,
}

#[derive(Debug)]
pub enum CliError {
    /// A real `cp` flag that v1 deliberately does not implement.
    Unsupported(String),
    UnknownFlag(String),
    MissingOperand,
    Help,
}

/// `cp` flags that exist but are out of scope for v1. Failing loudly on these
/// beats silently ignoring them: an ignored `-p` yields a tree with wrong
/// ownership and no indication anything went wrong.
const UNSUPPORTED_SHORT: &[char] = &['p', 'i', 'n', 'a', 'c', 'l', 's', 'X', 'H', 'L', 'P'];

pub fn parse(args: Vec<OsString>) -> Result<Invocation, CliError> {
    let mut options = Options::default();
    let mut operands: Vec<PathBuf> = Vec::new();
    let iter = args.into_iter();
    let mut only_operands = false;

    for arg in iter {
        if only_operands {
            operands.push(PathBuf::from(arg));
            continue;
        }

        let text = arg.to_string_lossy().into_owned();

        if text == "--" {
            only_operands = true;
            continue;
        }

        // BSD getopt does not permute: the first operand ends option parsing.
        let is_option = text.starts_with('-') && text != "-";
        if !is_option {
            only_operands = true;
            operands.push(PathBuf::from(arg));
            continue;
        }

        if text.starts_with("--") {
            if text == "--help" {
                return Err(CliError::Help);
            }
            match text.strip_prefix("--progress=") {
                Some("auto") => options.progress = ProgressMode::Auto,
                Some("force") => options.progress = ProgressMode::Force,
                Some("none") => options.progress = ProgressMode::None,
                _ => return Err(CliError::UnknownFlag(text)),
            }
            continue;
        }

        for c in text.chars().skip(1) {
            match c {
                'R' | 'r' => options.recursive = true,
                'f' => options.force = true,
                'v' => options.verbose = true,
                // Free to claim: `cp -h` is an illegal option, not a flag.
                'h' => return Err(CliError::Help),
                _ if UNSUPPORTED_SHORT.contains(&c) => {
                    return Err(CliError::Unsupported(format!("-{c}")))
                }
                _ => return Err(CliError::UnknownFlag(format!("-{c}"))),
            }
        }
    }

    if operands.len() < 2 {
        return Err(CliError::MissingOperand);
    }

    let dest = operands.pop().expect("length checked above");
    Ok(Invocation {
        options,
        sources: operands,
        dest,
    })
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Unsupported(flag) => write!(
                f,
                "cprogress: {flag} is not supported\n{USAGE}"
            ),
            CliError::UnknownFlag(flag) => {
                write!(f, "cprogress: illegal option {flag}\n{USAGE}")
            }
            // Matches cp: a bare invocation prints usage and nothing else.
            CliError::MissingOperand => write!(f, "{USAGE}"),
            CliError::Help => write!(f, "{USAGE}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> Invocation {
        parse(args.iter().map(OsString::from).collect()).expect("expected success")
    }

    fn parse_err(args: &[&str]) -> CliError {
        parse(args.iter().map(OsString::from).collect()).expect_err("expected failure")
    }

    #[test]
    fn parses_separate_flags() {
        let inv = parse_ok(&["-R", "-f", "src", "dst"]);
        assert!(inv.options.recursive);
        assert!(inv.options.force);
        assert!(!inv.options.verbose);
        assert_eq!(inv.sources, vec![PathBuf::from("src")]);
        assert_eq!(inv.dest, PathBuf::from("dst"));
    }

    #[test]
    fn parses_clustered_flags() {
        let inv = parse_ok(&["-Rfv", "src", "dst"]);
        assert!(inv.options.recursive);
        assert!(inv.options.force);
        assert!(inv.options.verbose);
    }

    #[test]
    fn lowercase_r_is_recursive_too() {
        assert!(parse_ok(&["-r", "src", "dst"]).options.recursive);
    }

    #[test]
    fn multiple_sources_take_the_last_operand_as_dest() {
        let inv = parse_ok(&["-R", "a", "b", "c"]);
        assert_eq!(inv.sources, vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(inv.dest, PathBuf::from("c"));
    }

    #[test]
    fn double_dash_terminates_option_parsing() {
        let inv = parse_ok(&["--", "-weird-name", "dst"]);
        assert_eq!(inv.sources, vec![PathBuf::from("-weird-name")]);
        assert!(!inv.options.recursive);
    }

    #[test]
    fn option_parsing_stops_at_the_first_operand_like_bsd() {
        // BSD getopt does not permute: `-R` here is an operand, not a flag.
        let inv = parse_ok(&["a", "b", "-R"]);
        assert!(!inv.options.recursive);
        assert_eq!(inv.sources, vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(inv.dest, PathBuf::from("-R"));
    }

    #[test]
    fn bare_dash_is_an_operand() {
        let inv = parse_ok(&["-", "dst"]);
        assert_eq!(inv.sources, vec![PathBuf::from("-")]);
    }

    #[test]
    fn progress_mode_defaults_to_auto_and_can_be_set() {
        assert_eq!(parse_ok(&["a", "b"]).options.progress, ProgressMode::Auto);
        assert_eq!(
            parse_ok(&["--progress=none", "a", "b"]).options.progress,
            ProgressMode::None
        );
        assert_eq!(
            parse_ok(&["--progress=force", "a", "b"]).options.progress,
            ProgressMode::Force
        );
    }

    #[test]
    fn known_cp_flags_we_do_not_support_are_named_in_the_error() {
        match parse_err(&["-p", "a", "b"]) {
            CliError::Unsupported(f) => assert_eq!(f, "-p"),
            other => panic!("wrong error: {other:?}"),
        }
        assert!(matches!(parse_err(&["-i", "a", "b"]), CliError::Unsupported(_)));
        assert!(matches!(parse_err(&["-n", "a", "b"]), CliError::Unsupported(_)));
        assert!(matches!(parse_err(&["-a", "a", "b"]), CliError::Unsupported(_)));
    }

    #[test]
    fn unsupported_flag_inside_a_cluster_is_caught() {
        assert!(matches!(parse_err(&["-Rp", "a", "b"]), CliError::Unsupported(_)));
    }

    #[test]
    fn unknown_flags_are_rejected() {
        assert!(matches!(parse_err(&["-z", "a", "b"]), CliError::UnknownFlag(_)));
        assert!(matches!(parse_err(&["--bogus", "a", "b"]), CliError::UnknownFlag(_)));
        assert!(matches!(
            parse_err(&["--progress=bogus", "a", "b"]),
            CliError::UnknownFlag(_)
        ));
    }

    #[test]
    fn too_few_operands_is_a_usage_error() {
        assert!(matches!(parse_err(&[]), CliError::MissingOperand));
        assert!(matches!(parse_err(&["a"]), CliError::MissingOperand));
        assert!(matches!(parse_err(&["-R"]), CliError::MissingOperand));
    }

    #[test]
    fn help_is_its_own_outcome() {
        assert!(matches!(parse_err(&["--help"]), CliError::Help));
        assert!(matches!(parse_err(&["-h"]), CliError::Help));
        // Verified: `cp -h` is an illegal option, so -h is free to claim.
        assert!(matches!(parse_err(&["-Rh", "a", "b"]), CliError::Help));
    }
}
