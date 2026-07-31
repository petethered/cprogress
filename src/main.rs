use cprogress::cli::{self, CliError, ProgressMode};
use cprogress::progress::bar::Bar;
use cprogress::progress::{Null, Progress};
use cprogress::{engine, help, plan, term};
use std::env;
use std::ffi::OsString;
use std::io;
use std::os::unix::io::AsRawFd;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

const EXIT_OK: u8 = 0;
const EXIT_FAILURE: u8 = 1;
/// `EX_USAGE` from sysexits.h — what BSD `cp` returns for a bad invocation.
const EXIT_USAGE: u8 = 64;
const EXIT_INTERRUPTED: u8 = 130;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigint(_signal: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

fn install_sigint_handler() {
    unsafe {
        libc::signal(libc::SIGINT, handle_sigint as libc::sighandler_t);
    }
}

fn main() -> ExitCode {
    install_sigint_handler();

    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let invocation = match cli::parse(args) {
        Ok(inv) => inv,
        // -h/--help goes to stdout and succeeds; everything else is a usage error.
        Err(CliError::Help) => {
            print!("{}", help::MANUAL);
            return ExitCode::from(EXIT_OK);
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let stderr = io::stderr();
    let use_bar = match invocation.options.progress {
        ProgressMode::Auto => term::is_tty(stderr.as_raw_fd()),
        ProgressMode::Force => true,
        ProgressMode::None => false,
    };

    // The bar and every diagnostic go to stderr; -v lines go to stdout, which
    // is where cp puts them.
    let mut boxed: Box<dyn Progress> = if use_bar {
        let fd = stderr.as_raw_fd();
        Box::new(Bar::new(
            io::stderr(),
            io::stdout(),
            Box::new(move || term::width_or_fallback(fd)),
        ))
    } else {
        Box::new(Null::new(io::stderr(), io::stdout()))
    };
    let progress = boxed.as_mut();

    let built = {
        let mut tick = |files: usize, bytes: u64| progress.scan_tick(files, bytes);
        plan::build(
            &invocation.sources,
            &invocation.dest,
            &invocation.options,
            &mut tick,
        )
    };

    let plan = match built {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };

    let mut failed = !plan.errors.is_empty();
    for line in &plan.errors {
        progress.note(line);
    }

    progress.start(plan.total_bytes, plan.file_count);
    let outcome = engine::run(&plan, &invocation.options, progress, &INTERRUPTED);
    progress.finish();

    failed |= outcome.failed;

    if outcome.interrupted {
        ExitCode::from(EXIT_INTERRUPTED)
    } else if failed {
        ExitCode::from(EXIT_FAILURE)
    } else {
        ExitCode::from(EXIT_OK)
    }
}
