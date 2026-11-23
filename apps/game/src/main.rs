use bevy::app::AppExit;
use bevy_destruction::{parse_options, run};

fn main() -> AppExit {
    let options = parse_options(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });

    run(options)
}
