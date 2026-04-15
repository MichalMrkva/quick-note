# qn — quick note

Minimal CLI for daily notes stored in SQLite.

## Install

```bash
cargo install --path .
```

## Usage

```bash
# Add a note
qn "call the doctor"
# Note added.

# List today's notes
qn l
# [ ] call the doctor
# [x] buy groceries

# List today's notes with IDs
qn l i
# (3) [ ] call the doctor
# (2) [x] buy groceries

# List notes from the last N days
qn l s 3
# --------TODAY::15.04.2026--------
# [ ] call the doctor
# [x] buy groceries
# ------Monday::13.04.2026---------
# [x] review PR

# List notes from the last N days with IDs
qn l s 3 i
# --------TODAY::15.04.2026--------
# (3) [ ] call the doctor
# (2) [x] buy groceries
# ------Monday::13.04.2026---------
# (1) [x] review PR

# Toggle a note done/undone
qn d 3
# Toggled note 3.
```
