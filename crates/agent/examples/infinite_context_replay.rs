use agent::cache_replay::{ReplayOptions, ReplayPrefix, replay_json};
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const HELP: &str = "Offline InfiniteContext replay (never calls models or executes tools).\n\
Usage: infinite_context_replay --input FILE [--report FILE]\n\
  --summary-bytes 256|512       Extractive placeholder budget (default 512)\n\
  --ttl-seconds N              Simulated cache TTL (default 300)\n\
  --request-gap-seconds N      Gap after each recorded Agent request (default 1)\n\
  --pause-after-request N      Pause after global recorded request N (one-based)\n\
  --pause-seconds N            Simulated pause (default 600)\n\
  --shared-model              Share turn/summary cache model scope\n\
  --repeats N                 Artificial independent-thread repetitions (default 1)\n\
  --min-cache-bytes N          Byte eligibility proxy, NOT token minimum (default 0)\n\
  --prefix-json FILE          Optional {\"system\":[\"...\"],\"tools\":[{\"name\":\"...\",\"description\":\"...\",\"input_schema\":{}}]}\n\
Input: uncompressed v0.3.0 conversation JSON or v0.2.0 SerializedThread JSON.\n\
The report is always JSON on stdout; --report creates a new file, never overwrites one.";

#[derive(Clone)]
struct Args {
    input: PathBuf,
    report: Option<PathBuf>,
    prefix_json: Option<PathBuf>,
    options: ReplayOptions,
}

impl Args {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut args = Self {
            input: PathBuf::new(),
            report: None,
            prefix_json: None,
            options: ReplayOptions::default(),
        };
        let mut seen = BTreeSet::new();
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            if flag == "--help" || flag == "-h" {
                return Ok(None);
            }
            ensure!(seen.insert(flag.clone()), "duplicate CLI option");
            if flag == "--shared-model" {
                args.options.shared_model = true;
                continue;
            }
            ensure!(
                matches!(
                    flag.as_str(),
                    "--input"
                        | "--report"
                        | "--prefix-json"
                        | "--summary-bytes"
                        | "--ttl-seconds"
                        | "--request-gap-seconds"
                        | "--pause-after-request"
                        | "--pause-seconds"
                        | "--repeats"
                        | "--min-cache-bytes"
                ),
                "unknown CLI option; use --help"
            );
            let value = arguments.next().context("missing CLI option value")?;
            match flag.as_str() {
                "--input" => args.input = value.into(),
                "--report" => args.report = Some(value.into()),
                "--prefix-json" => args.prefix_json = Some(value.into()),
                "--summary-bytes" => {
                    args.options.summary_bytes =
                        value.parse().context("invalid summary byte budget")?
                }
                "--ttl-seconds" => {
                    args.options.ttl_seconds = value.parse().context("invalid TTL")?
                }
                "--request-gap-seconds" => {
                    args.options.request_gap_seconds =
                        value.parse().context("invalid request gap")?
                }
                "--pause-after-request" => {
                    args.options.pause_after_request =
                        Some(value.parse().context("invalid pause request index")?)
                }
                "--pause-seconds" => {
                    args.options.pause_seconds = value.parse().context("invalid pause duration")?
                }
                "--repeats" => {
                    args.options.repeats = value.parse().context("invalid repetition count")?
                }
                "--min-cache-bytes" => {
                    args.options.min_cache_bytes =
                        value.parse().context("invalid cache byte minimum")?
                }
                _ => bail!("unknown CLI option"),
            }
        }
        ensure!(!args.input.as_os_str().is_empty(), "--input is required");
        args.options.validate()?;
        Ok(Some(args))
    }
}

fn run(args: &Args) -> Result<Value> {
    let bytes = fs::read(&args.input).context("read input JSON")?;
    let mut options = args.options.clone();
    if let Some(path) = &args.prefix_json {
        options.prefix = Some(
            serde_json::from_slice::<ReplayPrefix>(&fs::read(path).context("read prefix JSON")?)
                .map_err(|_| {
                    anyhow::anyhow!(
                        "invalid prefix JSON; expected system string array and request-tool schemas"
                    )
                })?,
        );
    }
    serde_json::to_value(replay_json(&bytes, &options, &AtomicBool::new(false))?)
        .context("serialize replay report")
}

fn write_report(path: &Path, report: &Value) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("create report file (must not already exist)")?;
    serde_json::to_writer_pretty(&mut file, report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    let result = Args::parse(std::env::args().skip(1)).and_then(|args| {
        let Some(args) = args else {
            return Ok(json!({"help": HELP, "offline_only": true}));
        };
        let report = run(&args)?;
        if let Some(path) = &args.report {
            write_report(path, &report)?;
        }
        Ok(report)
    });
    let (report, code) = match result {
        Ok(report) => (report, std::process::ExitCode::SUCCESS),
        Err(error) => (
            json!({"benchmark": "infinite_context_offline_replay", "offline_only": true, "error": error.to_string(), "live_model_calls": 0, "tool_executions": 0, "network_calls": 0}),
            std::process::ExitCode::FAILURE,
        ),
    };
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    if let Err(error) = serde_json::to_writer_pretty(&mut stdout, &report)
        .and_then(|()| stdout.write_all(b"\n").map_err(serde_json::Error::io))
    {
        eprintln!("Cannot write offline replay JSON report: {error}");
        return std::process::ExitCode::FAILURE;
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn cli_defaults_match_library_defaults() -> Result<()> {
        let args =
            Args::parse(["--input".into(), "unused.json".into()])?.context("expected arguments")?;
        let defaults = ReplayOptions::default();
        assert_eq!(args.options.summary_bytes, defaults.summary_bytes);
        assert_eq!(args.options.ttl_seconds, defaults.ttl_seconds);
        assert_eq!(
            args.options.request_gap_seconds,
            defaults.request_gap_seconds
        );
        assert_eq!(
            args.options.pause_after_request,
            defaults.pause_after_request
        );
        assert_eq!(args.options.pause_seconds, defaults.pause_seconds);
        assert_eq!(args.options.shared_model, defaults.shared_model);
        assert_eq!(args.options.repeats, defaults.repeats);
        assert_eq!(args.options.min_cache_bytes, defaults.min_cache_bytes);
        assert!(args.options.prefix.is_none());
        Ok(())
    }

    #[test]
    fn cli_flags_and_validation_are_preserved() -> Result<()> {
        let args = Args::parse(
            [
                "--input",
                "input.json",
                "--report",
                "report.json",
                "--prefix-json",
                "prefix.json",
                "--summary-bytes",
                "256",
                "--ttl-seconds",
                "10",
                "--request-gap-seconds",
                "2",
                "--pause-after-request",
                "3",
                "--pause-seconds",
                "4",
                "--shared-model",
                "--repeats",
                "5",
                "--min-cache-bytes",
                "6",
            ]
            .map(String::from),
        )?
        .context("expected arguments")?;
        assert_eq!(args.input, PathBuf::from("input.json"));
        assert_eq!(args.report, Some(PathBuf::from("report.json")));
        assert_eq!(args.prefix_json, Some(PathBuf::from("prefix.json")));
        assert_eq!(args.options.summary_bytes, 256);
        assert_eq!(args.options.ttl_seconds, 10);
        assert_eq!(args.options.request_gap_seconds, 2);
        assert_eq!(args.options.pause_after_request, Some(3));
        assert_eq!(args.options.pause_seconds, 4);
        assert!(args.options.shared_model);
        assert_eq!(args.options.repeats, 5);
        assert_eq!(args.options.min_cache_bytes, 6);
        assert!(Args::parse(["--help".into()])?.is_none());
        for flags in [
            vec![],
            vec!["--input"],
            vec!["--unknown"],
            vec!["--input", "x", "--input", "y"],
            vec!["--input", "x", "--summary-bytes", "1"],
            vec!["--input", "x", "--repeats", "0"],
            vec!["--input", "x", "--pause-after-request", "0"],
        ] {
            assert!(Args::parse(flags.into_iter().map(String::from)).is_err());
        }
        Ok(())
    }

    #[test]
    fn report_does_not_overwrite() -> Result<()> {
        let path = std::env::temp_dir().join(format!("zed-replay-report-{}.json", Uuid::new_v4()));
        let result = (|| {
            write_report(&path, &json!({"original": true}))?;
            let before = fs::read(&path)?;
            assert!(write_report(&path, &json!({"original": false})).is_err());
            assert_eq!(before, fs::read(&path)?);
            Ok(())
        })();
        fs::remove_file(path)?;
        result
    }
}
