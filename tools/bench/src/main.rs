//! Tabby v2 micro-benchmark harness.
//!
//! Sub-commands:
//!
//! - `cold-start`   : spawns `tabby` N times, records time from process spawn
//!   to first byte of the `core.version` response on stdout.
//! - `rpc-loopback` : spawns `tabby` once, fires N `core.version` calls back
//!   to back, reports median + p95 latency.
//! - `local-shell`  : opens N local-shell sessions sequentially, measures
//!   `session.openLocal` latency.
//!
//! All metrics print as JSON so CI can parse them.

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Parser)]
#[command(version, about = "Tabby v2 benchmark harness")]
struct Cli {
    /// Path to the tabby binary.
    #[arg(long, default_value = "target/release/tabby")]
    tabby: String,
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

async fn cold_start(tabby: &str, iters: usize) -> Result<Stats> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let started = Instant::now();
        let mut child = Command::new(tabby)
            .env("RUST_LOG", "off")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawn tabby")?;
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

async fn rpc_loopback(tabby: &str, iters: usize) -> Result<Stats> {
    let mut child = Command::new(tabby)
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

async fn local_shell(tabby: &str, iters: usize) -> Result<Stats> {
    let mut child = Command::new(tabby)
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
    let s = match cli.cmd {
        Cmd::ColdStart { iterations } => cold_start(&cli.tabby, iterations).await?,
        Cmd::RpcLoopback { iterations } => rpc_loopback(&cli.tabby, iterations).await?,
        Cmd::LocalShell { iterations } => local_shell(&cli.tabby, iterations).await?,
    };
    println!("{}", serde_json::to_string_pretty(&s)?);
    Ok(())
}
