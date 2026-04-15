use chrono::{Duration, Local, NaiveDate};
use colored::*;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use rusqlite::{params, Connection};
use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
    process,
};

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
}

fn cmd_remove(id: i64) {
    let conn = open_db();
    let changed = conn
        .execute("DELETE FROM notes WHERE id = ?1", params![id])
        .unwrap_or_else(|e| {
            eprintln!("Failed to remove: {e}");
            process::exit(1);
        });
    if changed == 0 {
        eprintln!("No note with id={id}");
        process::exit(1);
    }
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

// ── Interactive mode ──────────────────────────────────────────────────────────

enum AppMode {
    Notes {
        date: NaiveDate,
        notes: Vec<Note>,
        cursor: usize,
    },
    DaySelect {
        dates: Vec<NaiveDate>,
        cursor: usize,
    },
}

struct RawModeGuard;
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn get_notes_for_date(conn: &Connection, date: NaiveDate) -> Vec<Note> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let today = Local::now().date_naive();
    let mut stmt = conn
        .prepare(
            "SELECT id, text, done, created_at \
             FROM notes \
             WHERE created_at = ?1 \
             ORDER BY id ASC",
        )
        .unwrap_or_else(|e| {
            eprintln!("Query prepare failed: {e}");
            process::exit(1);
        });
    stmt.query_map(params![date_str], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i32>(2)?,
            row.get::<_, String>(3)?,
        ))
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .map(|(id, text, done, ds)| {
        let d = NaiveDate::parse_from_str(&ds, "%Y-%m-%d").unwrap_or(today);
        Note { id, text, done: done != 0, date: d }
    })
    .collect()
}

fn get_all_dates(conn: &Connection) -> Vec<NaiveDate> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT created_at FROM notes ORDER BY created_at DESC")
        .unwrap_or_else(|e| {
            eprintln!("Query prepare failed: {e}");
            process::exit(1);
        });
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .filter_map(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .collect()
}

fn toggle_note_in_db(conn: &Connection, id: i64) {
    conn.execute(
        "UPDATE notes \
         SET done = CASE WHEN done = 0 THEN 1 ELSE 0 END \
         WHERE id = ?1",
        params![id],
    )
    .unwrap_or_else(|e| {
        eprintln!("Failed to toggle: {e}");
        process::exit(1);
    });
}

fn draw_interactive(mode: &AppMode, today: NaiveDate) {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All)
    )
    .unwrap();

    match mode {
        AppMode::Notes { date, notes, cursor } => {
            let label = if *date == today {
                "TODAY".to_string()
            } else {
                date.format("%A").to_string()
            };
            let formatted = date.format("%d.%m.%Y").to_string();
            print!("=== {} :: {} ===\r\n", label, formatted);
            print!("{}\r\n", "─".repeat(40));

            if notes.is_empty() {
                print!("  (no notes)\r\n");
            } else {
                for (i, note) in notes.iter().enumerate() {
                    let marker = if note.done {
                        "[x]".green().to_string()
                    } else {
                        "[ ]".red().to_string()
                    };
                    let prefix = if i == *cursor { ">" } else { " " };
                    if i == *cursor {
                        print!("{} {} {}\r\n", prefix, marker, note.text.bold());
                    } else {
                        print!("{} {} {}\r\n", prefix, marker, note.text);
                    }
                }
            }

            print!("\r\n");
            print!("↑↓ move  Enter toggle  Backspace select day  q quit\r\n");
        }
        AppMode::DaySelect { dates, cursor } => {
            print!("=== Select Day ===\r\n");
            print!("{}\r\n", "─".repeat(40));

            if dates.is_empty() {
                print!("  (no days with notes)\r\n");
            } else {
                for (i, date) in dates.iter().enumerate() {
                    let label = if *date == today {
                        "TODAY".to_string()
                    } else {
                        date.format("%A").to_string()
                    };
                    let formatted = date.format("%d.%m.%Y").to_string();
                    let prefix = if i == *cursor { ">" } else { " " };
                    if i == *cursor {
                        print!("{} {} :: {}\r\n", prefix, label.bold(), formatted.bold());
                    } else {
                        print!("{} {} :: {}\r\n", prefix, label, formatted);
                    }
                }
            }

            print!("\r\n");
            print!("↑↓ move  Enter select  q quit\r\n");
        }
    }

    stdout.flush().unwrap();
}

fn cmd_interactive() {
    let conn = open_db();
    let today = Local::now().date_naive();

    let notes = get_notes_for_date(&conn, today);
    let mut mode = AppMode::Notes {
        date: today,
        notes,
        cursor: 0,
    };

    terminal::enable_raw_mode().unwrap_or_else(|e| {
        eprintln!("Cannot enable raw mode: {e}");
        process::exit(1);
    });
    let _guard = RawModeGuard;

    loop {
        draw_interactive(&mode, today);

        let ev = event::read().unwrap_or_else(|e| {
            eprintln!("Event read failed: {e}");
            process::exit(1);
        });

        if let Event::Key(key) = ev {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,

                KeyCode::Up => match &mut mode {
                    AppMode::Notes { cursor, .. } => {
                        if *cursor > 0 {
                            *cursor -= 1;
                        }
                    }
                    AppMode::DaySelect { cursor, .. } => {
                        if *cursor > 0 {
                            *cursor -= 1;
                        }
                    }
                },

                KeyCode::Down => match &mut mode {
                    AppMode::Notes { cursor, notes, .. } => {
                        if *cursor + 1 < notes.len() {
                            *cursor += 1;
                        }
                    }
                    AppMode::DaySelect { cursor, dates, .. } => {
                        if *cursor + 1 < dates.len() {
                            *cursor += 1;
                        }
                    }
                },

                KeyCode::Enter => {
                    let new_mode = match &mode {
                        AppMode::Notes { cursor, notes, date } => {
                            notes.get(*cursor).map(|note| {
                                toggle_note_in_db(&conn, note.id);
                                let new_notes = get_notes_for_date(&conn, *date);
                                AppMode::Notes {
                                    date: *date,
                                    notes: new_notes,
                                    cursor: *cursor,
                                }
                            })
                        }
                        AppMode::DaySelect { cursor, dates } => {
                            dates.get(*cursor).map(|&selected_date| {
                                let new_notes = get_notes_for_date(&conn, selected_date);
                                AppMode::Notes {
                                    date: selected_date,
                                    notes: new_notes,
                                    cursor: 0,
                                }
                            })
                        }
                    };
                    if let Some(m) = new_mode {
                        mode = m;
                    }
                }

                KeyCode::Backspace => {
                    if let AppMode::Notes { date, .. } = &mode {
                        let cur_date = *date;
                        let dates = get_all_dates(&conn);
                        let cursor = dates.iter().position(|d| *d == cur_date).unwrap_or(0);
                        mode = AppMode::DaySelect { dates, cursor };
                    }
                }

                _ => {}
            }
        }
    }

    // _guard drops here → disable_raw_mode() is called
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )
    .unwrap();
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
  qn d <id>          Toggle done/undone on note <id>
  qn rm <id>         Remove note by id
  qn int             Interactive mode (today's notes)"
    );
    process::exit(1);
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    match args.as_slice() {
        // qn "some note text"
        [text] if text != "l" && text != "d" && text != "int" => {
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

        // qn rm <id>
        [rm, id_arg] if rm == "rm" => {
            let id: i64 = id_arg.parse().unwrap_or_else(|_| {
                eprintln!("Expected a numeric id, got: {id_arg}");
                process::exit(1);
            });
            cmd_remove(id);
        }

        // qn int
        [i] if i == "int" => {
            cmd_interactive();
        }

        _ => usage(),
    }
}
