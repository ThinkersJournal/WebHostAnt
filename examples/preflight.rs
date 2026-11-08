//! Check this machine before a Cloudflare container deploy: `cargo run --example preflight`. Reads only; prints no email, account or token.

use std::time::Duration;
use webhostant::cloudflare::ProcessRunner;
use webhostant::preflight::Preflight;

fn main() {
    let wrangler = ProcessRunner::with_timeout(Duration::from_secs(60));
    let docker = ProcessRunner::program("docker", Duration::from_secs(60));
    let report = Preflight::new(wrangler, docker).with_containers().run();
    print!("{report}");
    std::process::exit(if report.ready() { 0 } else { 1 });
}
