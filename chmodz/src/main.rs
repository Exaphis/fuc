use std::path::PathBuf;

use clap::{ArgAction, Parser, ValueHint};
use error_stack::Report;
use fuc_engine::{ChmodMode, ChmodOp, Error};

/// A zippy alternative to `chmod`, a tool to change file mode bits of files and directories
#[derive(Parser, Debug)]
#[command(version, author = "Alex Saveau (@SUPERCILEX), Kevin Wu (@Exaphis")]
#[command(infer_subcommands = true, infer_long_args = true)]
#[command(disable_help_flag = true)]
#[command(arg_required_else_help = true)]
#[command(max_term_width = 100)]
#[cfg_attr(test, command(help_expected = true))]
struct Chmodz {
    /// The desired mode (octal or symbolic)
    #[arg(required = true)]
    mode: String,

    /// The files and/or directories to have their mode changed
    #[arg(required = true)]
    #[arg(value_hint = ValueHint::AnyPath)]
    files: Vec<PathBuf>,

    #[arg(short, long, short_alias = '?', global = true)]
    #[arg(action = ArgAction::Help, help = "Print help (use `--help` for more detail)")]
    #[arg(long_help = "Print help (use `-h` for a summary)")]
    help: Option<bool>,
}

#[derive(thiserror::Error, Debug)]
enum CliError {
    #[error("{0}")]
    Wrapper(String),
}

#[cfg(feature = "trace")]
#[global_allocator]
static GLOBAL: tracy_client::ProfiledAllocator<std::alloc::System> =
    tracy_client::ProfiledAllocator::new(std::alloc::System, 100);

fn main() -> error_stack::Result<(), CliError> {
    #[cfg(not(debug_assertions))]
    error_stack::Report::install_debug_hook::<std::panic::Location>(|_, _| {});

    #[cfg(feature = "trace")]
    {
        use tracing_subscriber::{
            fmt::format::DefaultFields, layer::SubscriberExt, util::SubscriberInitExt,
        };

        #[derive(Default)]
        struct Config(DefaultFields);

        impl tracing_tracy::Config for Config {
            type Formatter = DefaultFields;

            fn formatter(&self) -> &Self::Formatter {
                &self.0
            }

            fn stack_depth(&self, _: &tracing::Metadata<'_>) -> u16 {
                32
            }

            fn format_fields_in_zone_name(&self) -> bool {
                false
            }
        }

        tracing_subscriber::registry()
            .with(tracing_tracy::TracyLayer::new(Config::default()))
            .init();
    };

    let args = Chmodz::parse();
    let mode = args.mode.clone();

    chmod(args).map_err(|e| {
        let wrapper = CliError::Wrapper(format!("{e}"));
        match e {
            Error::Io { error, context } => Report::from(error)
                .attach_printable(context)
                .change_context(wrapper),
            Error::NotFound { file: _ } => {
                Report::from(wrapper).attach_printable("Use --force to ignore.")
            }
            Error::FileMode(error) => Report::from(CliError::Wrapper(format!("Invalid file mode '{mode}': {error}"))),
            Error::PreserveRoot | Error::Join | Error::BadPath | Error::Internal => {
                Report::from(wrapper)
            }
            Error::AlreadyExists { file: _ } => unreachable!(),
        }
    })
}

fn chmod(
    Chmodz {
        files,
        mode,
        help: _,
    }: Chmodz,
) -> Result<(), Error> {
    ChmodOp::builder()
        .files(files.into_iter())
        .mode(ChmodMode::new(mode.as_str()))
        .build()
        .run()
}

#[cfg(test)]
mod cli_tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn verify_app() {
        Chmodz::command().debug_assert();
    }

    #[test]
    fn help_for_review() {
        supercilex_tests::help_for_review(Chmodz::command());
    }
}
