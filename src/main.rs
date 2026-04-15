use chrono::{Duration, Local, NaiveDate};
use colored::*;
use rusqlite::{params, Connection};
use std::{env, fs, path::PathBuf, process};

// ── Storage ───────────────────────────────────────────────────────────────────

fn open_db() -> Connection {
    let dir: PathBuf = {
        let home = env::var("HOME").unwrap_or_else(|_| {
            eprintln!("HOME not set");
            process::exit(1);
        });
        PathBuf::from(home).join(".local/share/qn")
    };
    fs::create_dir_all(&dir).unwrap_or_else(|e| {
        eprintln!("Cannot create data dir {}: {e}", dir.display());
        process::exit(1);
    });
    let conn = Connection::open(dir.join("notes.db")).unwrap_or_else(|e| {
        eprintln!("Cannot open database: {e}");
        process::exit(1);
    });
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notes (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            text       TEXT    NOT NULL,
            done       INTEGER NOT NULL DEFAULT 0,
            created_at TEXT    NOT NULL
        );",
    )
    .unwrap_or_else(|e| {
        eprintln!("Cannot initialize schema: {e}");
        process::exit(1);
    });
    conn
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_add(text: &str) {
    let conn = open_db();
    let today = Local::now().format("%Y-%m-%d").to_string();
    conn.execute(
        "INSERT INTO notes (text, done, created_at) VALUES (?1, 0, ?2)",
        params![text, today],
    )
    .unwrap_or_else(|e| {
        eprintln!("Failed to add note: {e}");
        process::exit(1);
    });
    println!("Note added.");
}

fn cmd_toggle(id: i64) {
    let conn = open_db();
    let changed = conn
        .execute(
            "UPDATE notes \
             SET done = CASE WHEN done = 0 THEN 1 ELSE 0 END \
             WHERE id = ?1",
            params![id],
        )
        .unwrap_or_else(|e| {
            eprintln!("Failed to toggle: {e}");
            process::exit(1);
        });
    if changed == 0 {
        eprintln!("No note with id={id}");
        process::exit(1);
    }
    println!("Toggled note {id}.");
}

// ── Listing ───────────────────────────────────────────────────────────────────

struct Note {
    id: i64,
    text: String,
    done: bool,
    date: NaiveDate,
}

fn cmd_list(days: u32, show_ids: bool) {
    let conn = open_db();
    let today = Local::now().date_naive();
    let cutoff = today - Duration::days(i64::from(days) - 1);

    let mut stmt = conn
        .prepare(
            "SELECT id, text, done, created_at \
             FROM notes \
             WHERE created_at >= ?1 \
             ORDER BY created_at DESC, id DESC",
        )
        .unwrap_or_else(|e| {
            eprintln!("Query prepare failed: {e}");
            process::exit(1);
        });

    let notes: Vec<Note> = stmt
        .query_map(params![cutoff.format("%Y-%m-%d").to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap_or_else(|e| {
            eprintln!("Query failed: {e}");
            process::exit(1);
        })
        .filter_map(|r| r.ok())
        .map(|(id, text, done, date_str)| {
            let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or(today);
            Note {
                id,
                text,
                done: done != 0,
                date,
            }
        })
        .collect();

    if notes.is_empty() {
        println!("No notes found.");
        return;
    }

    let show_headers = days > 1;
    let mut current_date: Option<NaiveDate> = None;

    for note in &notes {
        if show_headers && current_date != Some(note.date) {
            current_date = Some(note.date);
            print_day_header(note.date, today);
        }
        print_note_line(note, show_ids);
    }
}

fn print_day_header(date: NaiveDate, today: NaiveDate) {
    let label = if date == today {
        "TODAY".to_string()
    } else {
        date.format("%A").to_string()
    };
    let formatted = date.format("%d.%m.%Y").to_string();
    let inner = format!("{}::{}", label, formatted);
    let total_width = 40_usize;
    let dashes = total_width.saturating_sub(inner.len());
    let left = dashes / 2;
    let right = dashes - left;
    println!("{}{}{}", "-".repeat(left), inner, "-".repeat(right));
}

fn print_note_line(note: &Note, show_ids: bool) {
    let marker = if note.done {
        "[x]".green().to_string()
    } else {
        "[ ]".red().to_string()
    };

    if show_ids {
        println!("({}) {} {}", note.id, marker, note.text);
    } else {
        println!("{} {}", marker, note.text);
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn usage() -> ! {
    eprintln!(
        "Usage:
  qn \"note text\"     Add a new note for today
  qn l               List today's notes
  qn l i             List today's notes with IDs
  qn l s <N>         List notes from the last N days
  qn l s <N> i       List last N days with IDs
  qn d <id>          Toggle done/undone on note <id>"
    );
    process::exit(1);
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    match args.as_slice() {
        // qn "some note text"
        [text] if text != "l" && text != "d" => {
            cmd_add(text);
        }

        // qn l
        [l] if l == "l" => {
            cmd_list(1, false);
        }

        // qn l i
        [l, i] if l == "l" && i == "i" => {
            cmd_list(1, true);
        }

        // qn l s <N>
        [l, s, n] if l == "l" && s == "s" => {
            let days: u32 = n.parse().unwrap_or_else(|_| {
                eprintln!("Expected a number for day count, got: {n}");
                process::exit(1);
            });
            cmd_list(days, false);
        }

        // qn l s <N> i
        [l, s, n, i] if l == "l" && s == "s" && i == "i" => {
            let days: u32 = n.parse().unwrap_or_else(|_| {
                eprintln!("Expected a number for day count, got: {n}");
                process::exit(1);
            });
            cmd_list(days, true);
        }

        // qn d <id>
        [d, id_arg] if d == "d" => {
            let id: i64 = id_arg.parse().unwrap_or_else(|_| {
                eprintln!("Expected a numeric id, got: {id_arg}");
                process::exit(1);
            });
            cmd_toggle(id);
        }

        _ => usage(),
    }
}
