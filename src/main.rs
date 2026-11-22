fn main() -> bevy::app::AppExit {
    bevy_destruction::run(std::env::args().skip(1))
}
