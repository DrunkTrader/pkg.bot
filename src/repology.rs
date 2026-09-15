//! Selectively restore tables from a Repology pg_dump stream.
//! This is quite hacky and is used to create the tables prior and them stream and selectively import
//! .sql COPY for these specific tables into the DB. The full DB is large and time consuming to load,
//! and we only care about a subset (~50%) of the data/tables, so this hack is worth it in production.
//! Any time there's a schema change in repology, this will have to be tweaked.
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    io::{self, BufRead, BufReader, Read},
    time::Instant,
};

use sqlx::{Connection, Executor, PgConnection};

use crate::models::REPOLOGY;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const TABLES: &[&str] = &[
    "repositories",
    "packages",
    "maintainers",
    "links",
    "repo_tracks",
    "repo_track_versions",
];
const BUFFER_SIZE: usize = 1024 * 1024;
const MAX_LINE: usize = 16 * 1024 * 1024;

/// Restore into a new db.
pub async fn run(dsn: &str) -> Result<()> {
    let stdin = io::stdin();
    restore(dsn, stdin.lock()).await
}

async fn restore(dsn: &str, input: impl Read) -> Result<()> {
    let start = Instant::now();
    let mut dump = Dump::new(input)?;
    let mut pg = PgConnection::connect(dsn).await?;

    // Use a TX so that a malformed dump doesn't leave half-baked schema.
    let mut tx = pg.begin().await?;
    tx.execute(REPOLOGY.set_timeouts.query.as_str()).await?;
    tx.execute(REPOLOGY.schema.query.as_str()).await?;

    // Get the expected COPY column lists from the schema.
    let columns: HashMap<String, String> =
        sqlx::query_as::<_, (String, String)>(&REPOLOGY.get_columns.query)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();

    let mut buf = Vec::with_capacity(BUFFER_SIZE);
    while let Some(header) = dump.next_table()? {
        let name = header.table.strip_prefix("repology.").unwrap_or("");
        if TABLES.contains(&name) {
            if columns.get(name) != Some(&header.columns) {
                return Err(format!(
                    "COPY columns changed for {}; update repology.sql",
                    header.table
                )
                .into());
            }

            log::info!("restoring {}", header.table);

            // Both the table name and column list have been checked against the schema.
            let mut copy = tx
                .copy_in_raw(&format!(
                    "COPY repology.{name} ({}) FROM STDIN",
                    columns[name]
                ))
                .await?;
            loop {
                let more = dump.read_data(&mut buf)?;
                if !buf.is_empty() {
                    copy.send(&*buf).await?;
                }
                if !more {
                    break;
                }
            }

            let rows = copy.finish().await?;
            log::info!("restored {}: {rows} rows", header.table);
        } else {
            while dump.read_data(&mut buf)? {}
        }
    }
    dump.require_tables(TABLES)?;

    log::info!("building indexes");
    tx.execute(REPOLOGY.create_indexes.query.as_str()).await?;
    tx.commit().await?;
    log::info!(
        "repology restore complete ({:.1}s)",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

struct CopyHeader {
    table: String,
    columns: String,
}

/// Reader for pg_dump's text COPY blocks in the repology dump.
struct Dump<R> {
    input: BufReader<R>,
    line: Vec<u8>,
    seen: HashSet<String>,
    in_copy: bool,
    complete: bool,
    restriction: Option<String>,
}

impl<R: Read> Dump<R> {
    fn new(input: R) -> Result<Self> {
        let mut dump = Self {
            input: BufReader::with_capacity(BUFFER_SIZE, input),
            line: Vec::new(),
            seen: HashSet::new(),
            in_copy: false,
            complete: false,
            restriction: None,
        };

        for expected in ["--", "-- PostgreSQL database dump", "--"] {
            dump.line.clear();
            read_line(&mut dump.input, &mut dump.line)?;
            if text(&dump.line)? != expected {
                return Err("expected an empty PostgreSQL database dump on stdin".into());
            }
        }

        Ok(dump)
    }

    fn next_table(&mut self) -> Result<Option<CopyHeader>> {
        if self.in_copy {
            return Err("unfinished COPY block".into());
        }
        let mut expected_table = None;
        loop {
            self.line.clear();
            if read_line(&mut self.input, &mut self.line)? == 0 {
                break;
            }

            let line = text(&self.line)?;
            if let Some(token) = line.strip_prefix("\\restrict ") {
                if self.restriction.is_some() || self.complete {
                    return Err("unexpected dump restriction marker".into());
                }
                self.restriction = Some(token.to_owned());
            } else if let Some(token) = line.strip_prefix("\\unrestrict ") {
                if !self.complete || self.restriction.as_deref() != Some(token) {
                    return Err("invalid dump end marker".into());
                }
                self.restriction = None;
            } else if line == "-- PostgreSQL database dump complete" {
                if expected_table.is_some() || self.complete {
                    return Err("unexpected dump completion marker".into());
                }
                self.complete = true;
            } else if let Some(meta) = line.strip_prefix("-- Data for Name: ") {
                if self.complete || expected_table.is_some() {
                    return Err("missing COPY block or data after dump completion".into());
                }
                let (name, rest) = meta
                    .split_once("; Type: TABLE DATA; Schema: ")
                    .ok_or("unsupported dump data section")?;
                let (schema, _) = rest.split_once("; Owner: ").ok_or("invalid data section")?;
                expected_table = Some(format!("{schema}.{name}"));
            } else if let Some(copy) = line.strip_prefix("COPY ") {
                let (table, columns) = copy
                    .strip_suffix(") FROM stdin;")
                    .and_then(|s| s.split_once(" ("))
                    .ok_or("unsupported COPY format; expected text COPY FROM stdin")?;
                if self.complete || expected_table.as_deref() != Some(table) {
                    return Err("COPY does not match its dump data section".into());
                }
                if self.seen.len() >= 1024 || !self.seen.insert(table.to_owned()) {
                    return Err(format!("duplicate table or too many COPY blocks: {table}").into());
                }
                self.in_copy = true;
                return Ok(Some(CopyHeader {
                    table: table.to_owned(),
                    columns: columns.to_owned(),
                }));
            } else if !line.is_empty()
                && line != "--"
                && (expected_table.is_some() || self.complete)
            {
                return Err("unexpected content in dump data section or footer".into());
            }

            // Ignore other schema SQL, functions, indexes, etc.
        }
        if !self.complete || expected_table.is_some() || self.restriction.is_some() {
            return Err("truncated dump: missing completion marker or COPY block".into());
        }
        Ok(None)
    }

    /// Create a batch with the complete rows excluding the COPY terminator char.
    fn read_data(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        if !self.in_copy {
            return Err("expected a COPY block".into());
        }
        buf.clear();
        while buf.len() < BUFFER_SIZE {
            let start = buf.len();
            let n = read_line(&mut self.input, buf)?;
            if n == 0 {
                return Err("truncated dump inside COPY data".into());
            }
            if matches!(&buf[start..], b"\\.\n" | b"\\.\r\n") {
                buf.truncate(start);
                self.in_copy = false;
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn require_tables(&self, tables: &[&str]) -> Result<()> {
        for table in tables {
            if !self.seen.contains(&format!("repology.{table}")) {
                return Err(format!("dump is missing repology.{table}").into());
            }
        }
        Ok(())
    }
}

fn read_line(input: &mut impl BufRead, buf: &mut Vec<u8>) -> Result<usize> {
    let start = buf.len();
    let n = input.take(MAX_LINE as u64 + 1).read_until(b'\n', buf)?;
    if n > MAX_LINE {
        return Err("dump line exceeds 16 MiB".into());
    }
    if n > 0 && buf.last() != Some(&b'\n') {
        return Err("truncated dump line".into());
    }
    // Metadata uses a separate buffer which the caller clears between reads.
    debug_assert_eq!(buf.len(), start + n);
    Ok(n)
}

fn text(line: &[u8]) -> Result<&str> {
    Ok(std::str::from_utf8(line)?.trim_end_matches(['\r', '\n']))
}
