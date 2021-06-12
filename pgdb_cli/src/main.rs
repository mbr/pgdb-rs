use std::{thread, time::Duration};

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Opts {}

fn main() -> anyhow::Result<()> {
    let opts = Opts::from_args();

    let _pg = pgdb::Postgres::build().start()?;

    thread::sleep(Duration::from_millis(500));
    println!();
    println!("Postgres is now running and ready to accept connections");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
