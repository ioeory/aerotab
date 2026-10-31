//! AeroTab micro-benchmark harness.
//!
//! Sub-commands:
//!
//! - `cold-start`       : spawns `aerotab` N times, records time from process spawn
//!   to first byte of the `core.version` response on stdout.
//! - `rpc-loopback`     : spawns `aerotab` once, fires N `core.version` calls back
//!   to back, reports median + p95 latency.
//! - `local-shell`      : opens N local-shell sessions sequentially, measures
//!   `session.openLocal` latency.
//! - `term-throughput`  : feeds a fixed ANSI corpus into the native Rust VT
//!   engine and reports lines/s + frame build latency (xterm path is UI-only;
//!   compare manually — default renderer remains canvas/xterm until parity).
//!
//! All metrics print as JSON so CI can parse them.

use std::process::Stdio;
use std::time::{Duration, Instant};

use aerotab_core::terminal::engine::{EngineConfig, TerminalEngine};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(version, about = "AeroTab benchmark harness")]
struct Cli {
    /// Path to the aerotab binary.
    #[arg(long, default_value = "target/release/aerotab")]
    aerotab: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    ColdStart {
        #[arg(long, default_value_t = 5)]
        iterations: usize,
    },
    RpcLoopback {
        #[arg(long, default_value_t = 1000)]
        iterations: usize,
    },
    LocalShell {
        #[arg(long, default_value_t = 10)]
        iterations: usize,
    },
    /// Native Rust VT engine throughput (feed + frame encode).
    TermThroughput {
        /// How many times to replay the ANSI corpus.
        #[arg(long, default_value_t = 50)]
        iterations: usize,
        /// Lines per corpus replay (each line is a short coloured ANSI sequence).
        #[arg(long, default_value_t = 500)]
        lines: usize,
        #[arg(long, default_value_t = 120)]
        cols: u16,
        #[arg(long, default_value_t = 40)]
        rows: u16,
    },
}

#[derive(Debug, Serialize)]
struct Stats {
    name: &'static str,
    n: usize,
    min_ms: f64,
    median_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    mean_ms: f64,
}

#[derive(Debug, Serialize)]
struct TermThroughputReport {
    name: &'static str,
    path: &'static str,
    iterations: usize,
    lines_per_iter: usize,
    cols: u16,
    rows: u16,
    total_lines: usize,
    elapsed_ms: f64,
    lines_per_sec: f64,
    frame_build: Stats,
    /// Pass gate from docs/perf-benchmark.md (≥ 500 lines/s).
    pass_lines_per_sec: bool,
}

fn stats(name: &'static str, mut samples: Vec<Duration>) -> Stats {
    samples.sort();
    let n = samples.len();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let median = ms(samples[n / 2]);
    let p95 = ms(samples[((n as f64) * 0.95) as usize]);
    let min = ms(samples[0]);
    let max = ms(samples[n - 1]);
    let mean = samples.iter().map(|d| ms(*d)).sum::<f64>() / n as f64;
    Stats {
        name,
        n,
        min_ms: min,
        median_ms: median,
        p95_ms: p95,
        max_ms: max,
        mean_ms: mean,
    }
}

fn ansi_corpus(lines: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines * 48);
    for i in 0..lines {
        let color = (i % 7) + 1;
        out.extend_from_slice(format!("\x1b[3{color}mline-{i:05}\x1b[0m\r\n").as_bytes());
    }
    out
}

fn term_throughput(iters: usize, lines: usize, cols: u16, rows: u16) -> TermThroughputReport {
    let corpus = ansi_corpus(lines);
    let (tx, _rx) = mpsc::unbounded_channel();
    let engine = TerminalEngine::new(
        EngineConfig {
            cols,
            rows,
            scrollback: 10_000,
        },
        tx,
    );

    let mut frame_samples = Vec::with_capacity(iters);
    let started = Instant::now();
    for _ in 0..iters {
        engine.feed(&corpus);
        let t0 = Instant::now();
        let _frame = engine.snapshot_frame();
        frame_samples.push(t0.elapsed());
    }
    let elapsed = started.elapsed();
    let total_lines = iters.saturating_mul(lines);
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    let lines_per_sec = if elapsed.as_secs_f64() > 0.0 {
        total_lines as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    TermThroughputReport {
        name: "term_throughput",
        path: "native-rust-vt",
        iterations: iters,
        lines_per_iter: lines,
        cols,
        rows,
        total_lines,
        elapsed_ms,
        lines_per_sec,
        frame_build: stats("frame_build", frame_samples),
        pass_lines_per_sec: lines_per_sec >= 500.0,
    }
}

async fn cold_start(aerotab: &str, iters: usize) -> Result<Stats> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let started = Instant::now();
        let mut child = Command::new(aerotab)
            .env("RUST_LOG", "off")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawn aerotab")?;
        {
            let stdin = child.stdin.as_mut().unwrap();
            stdin
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"core.version\"}\n")
                .await?;
            stdin.flush().await?;
        }
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        samples.push(started.elapsed());
        let _ = child.kill().await;
    }
    Ok(stats("cold_start_first_response", samples))
}

async fn rpc_loopback(aerotab: &str, iters: usize) -> Result<Stats> {
    let mut child = Command::new(aerotab)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let mut samples = Vec::with_capacity(iters);
    for i in 0..iters {
        let line = format!("{{\"jsonrpc\":\"2.0\",\"id\":{i},\"method\":\"core.version\"}}\n");
        let started = Instant::now();
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        let mut resp = String::new();
        reader.read_line(&mut resp).await?;
        samples.push(started.elapsed());
    }
    drop(stdin);
    let _ = child.kill().await;
    Ok(stats("rpc_loopback", samples))
}

async fn local_shell(aerotab: &str, iters: usize) -> Result<Stats> {
    let mut child = Command::new(aerotab)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let mut samples = Vec::with_capacity(iters);
    for i in 0..iters {
        let frame = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{i},\"method\":\"session.openLocal\",\"params\":{{}}}}\n"
        );
        let started = Instant::now();
        stdin.write_all(frame.as_bytes()).await?;
        stdin.flush().await?;
        let mut resp = String::new();
        reader.read_line(&mut resp).await?;
        samples.push(started.elapsed());
    }
    drop(stdin);
    let _ = child.kill().await;
    Ok(stats("local_shell_open", samples))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::ColdStart { iterations } => {
            let s = cold_start(&cli.aerotab, iterations).await?;
            println!("{}", serde_json::to_string_pretty(&s)?);
        }
        Cmd::RpcLoopback { iterations } => {
            let s = rpc_loopback(&cli.aerotab, iterations).await?;
            println!("{}", serde_json::to_string_pretty(&s)?);
        }
        Cmd::LocalShell { iterations } => {
            let s = local_shell(&cli.aerotab, iterations).await?;
            println!("{}", serde_json::to_string_pretty(&s)?);
        }
        Cmd::TermThroughput {
            iterations,
            lines,
            cols,
            rows,
        } => {
            let report = term_throughput(iterations, lines, cols, rows);
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.pass_lines_per_sec {
                anyhow::bail!(
                    "term-throughput below 500 lines/s (got {:.1})",
                    report.lines_per_sec
                );
            }
        }
    }
    Ok(())
}
