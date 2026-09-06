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

/// Initialize site templates from the site directory.
pub fn site_tpls(dir: &std::path::Path) -> Result<tera::Tera, Box<dyn std::error::Error>> {
    let mut tera =
        tera::Tera::new(&format!("{}/**/*.html", dir.display())).map_err(|e| err_chain(&e))?;
    tera.autoescape_on(vec![".html"]);
    register_filters(&mut tera);

    log::info!(
        "loaded {} templates from {}",
        tera.get_template_names().count(),
        dir.display()
    );

    Ok(tera)
}

/// Flatten an error and its sources into a single message. Tera's Display only
/// prints the outermost error, which is never the useful one.
pub fn err_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(cause) = src {
        msg.push_str(&format!(": {}", cause));
        src = cause.source();
    }

    msg
}

/// Register custom template filters.
fn register_filters(tera: &mut tera::Tera) {
    // {{ 1234567 | num }} => 1,234,567
    tera.register_filter(
        "num",
        |v: &tera::Value, _: &std::collections::HashMap<String, tera::Value>| {
            Ok(tera::Value::String(num(v.as_i64().unwrap_or(0))))
        },
    );

    // {{ 1258291 | bytes }} => 1.2 MB
    tera.register_filter(
        "bytes",
        |v: &tera::Value, _: &std::collections::HashMap<String, tera::Value>| {
            Ok(tera::Value::String(bytes(v.as_i64().unwrap_or(0))))
        },
    );
}

/// Format a number with thousands separators. Eg: 1234567 => 1,234,567
fn num(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);

    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.char_indices() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }

    out
}

/// Format a byte count in human readable form. Eg: 1258291 => 1.2 MB
fn bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut val = n as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", n, UNITS[unit])
    } else {
        format!("{:.1} {}", val, UNITS[unit])
    }
}
