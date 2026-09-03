/// Initialize logger.
pub fn logger() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .format(|buf, rec| {
            use std::io::Write;
            let level = if rec.level() != log::Level::Info {
                format!("[{}] ", rec.level())
            } else {
                String::new()
            };
            writeln!(
                buf,
                "{} {}:{} {}{}",
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f"),
                rec.file().unwrap_or("unknown"),
                rec.line().unwrap_or(0),
                level,
                rec.args()
            )
        })
        .init();
}
