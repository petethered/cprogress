use crate::help::USAGE;
use crate::msg::PROGRAM;
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

/// Both flag variants carry the bare option character, without its leading
/// dash, because that is the unit `cp`'s own messages are built from:
/// "illegal option -- z".
#[derive(Debug)]
pub enum CliError {
    /// A real `cp` flag that v1 deliberately does not implement.
    Unsupported(char),
    UnknownFlag(char),
    MissingOperand,
    Help,
}

/// `cp` flags that exist but are out of scope for v1. Failing loudly on these
/// beats silently ignoring them: an ignored `-p` yields a tree with wrong
/// ownership and no indication anything went wrong. Every documented BSD `cp`
/// option we do not implement belongs here, so none of them ever falls through
/// to the "illegal option" path a genuinely unknown letter gets.
pub(crate) const UNSUPPORTED_SHORT: &[char] = &[
    'p', 'i', 'n', 'a', 'c', 'l', 's', 'X', 'H', 'L', 'P', 'N', 'S', 'x',
];

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
                // `cp` has no long options at all, so its getopt reads the
                // second dash as the option character: `cp --bogus` reports
                // "illegal option -- -". Ours says the same.
                _ => return Err(CliError::UnknownFlag('-')),
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
                _ if UNSUPPORTED_SHORT.contains(&c) => return Err(CliError::Unsupported(c)),
                _ => return Err(CliError::UnknownFlag(c)),
            }
        }
    }

    if operands.len() < 2 {
        return Err(CliError::MissingOperand);
    }

    // Unreachable — the length check above guarantees a last element. Written
    // as a `let ... else` rather than `expect` to keep the no-panic rule.
    let Some(dest) = operands.pop() else {
        return Err(CliError::MissingOperand);
    };
    Ok(Invocation {
        options,
        sources: operands,
        dest,
    })
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Unsupported(c) => {
                write!(f, "{PROGRAM}: -{c} is not supported\n{USAGE}")
            }
            // Verbatim `cp`: "/bin/cp: illegal option -- z".
            CliError::UnknownFlag(c) => {
                write!(f, "{PROGRAM}: illegal option -- {c}\n{USAGE}")
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
            CliError::Unsupported(c) => assert_eq!(c, 'p'),
            other => panic!("wrong error: {other:?}"),
        }
        assert!(matches!(parse_err(&["-i", "a", "b"]), CliError::Unsupported(_)));
        assert!(matches!(parse_err(&["-n", "a", "b"]), CliError::Unsupported(_)));
        assert!(matches!(parse_err(&["-a", "a", "b"]), CliError::Unsupported(_)));
    }

    #[test]
    fn every_documented_cp_option_we_lack_is_unsupported_not_illegal() {
        // The full documented BSD option set, from `man cp` on macOS. Every
        // one we do not implement must report "is not supported", never
        // "illegal option" — the latter is for letters cp itself would
        // reject. -N, -S and -x were missing from UNSUPPORTED_SHORT.
        const ALL: &[char] = &[
            'H', 'L', 'P', 'R', 'f', 'i', 'l', 'n', 'p', 's', 'v', 'X', 'a', 'c', 'x', 'N', 'S',
        ];
        const IMPLEMENTED: &[char] = &['R', 'f', 'v'];
        for &c in ALL {
            if IMPLEMENTED.contains(&c) {
                continue;
            }
            let flag = format!("-{c}");
            match parse(vec![OsString::from(&flag), "a".into(), "b".into()]) {
                Err(CliError::Unsupported(got)) => assert_eq!(got, c),
                other => panic!("-{c} should be Unsupported, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_manual_lists_exactly_the_unsupported_flags() {
        // The constant and the documentation drifted apart once already; this
        // catches drift in both directions.
        let line = crate::help::MANUAL
            .lines()
            .find(|l| l.trim_start().starts_with("-p -i -n"))
            .expect("the manual's unsupported-options list");
        let listed: Vec<&str> = line.split_whitespace().collect();
        for c in UNSUPPORTED_SHORT {
            let flag = format!("-{c}");
            assert!(listed.contains(&flag.as_str()), "the manual omits {flag}");
        }
        assert_eq!(
            listed.len(),
            UNSUPPORTED_SHORT.len(),
            "the manual lists options we do accept: {listed:?}"
        );
    }

    #[test]
    fn illegal_option_text_matches_cp_exactly() {
        // /bin/cp -z a b  ->  "/bin/cp: illegal option -- z"
        let text = parse_err(&["-z", "a", "b"]).to_string();
        assert_eq!(
            text.lines().next(),
            Some("cprogress: illegal option -- z"),
            "got {text:?}"
        );
        // /bin/cp --bogus a b  ->  "/bin/cp: illegal option -- -"
        let text = parse_err(&["--bogus", "a", "b"]).to_string();
        assert_eq!(
            text.lines().next(),
            Some("cprogress: illegal option -- -"),
            "got {text:?}"
        );
        // The usage block follows the diagnostic, as cp's does.
        assert!(text.ends_with(USAGE), "usage missing: {text:?}");
    }

    #[test]
    fn unsupported_option_text_names_the_flag() {
        let text = parse_err(&["-p", "a", "b"]).to_string();
        assert_eq!(
            text.lines().next(),
            Some("cprogress: -p is not supported"),
            "got {text:?}"
        );
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
