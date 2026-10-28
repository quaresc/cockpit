//! The runbook parser and the cell writer on any text: neither panics, and a cell written then read back gives the value
//! written, every other cell and every other line as they were.

use std::path::Path;

use cockpit_runbook::book::{Step, parse_text};
use cockpit_runbook::table::{self, Column, clean, insert_step, set_cell};
use proptest::prelude::*;

const COLUMNS: [Column; 4] = [Column::Step, Column::Where, Column::Status, Column::Command];

/// Cell text with what makes a table hard: pipes, backslashes, backticks, emoji, spaces at either end.
fn cell() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 |\\\\`#:.✅⬜🚀é-]{0,16}"
}

/// A runbook whose table is written by hand, its cells padded by one to three spaces.
fn runbook() -> impl Strategy<Value = (String, Vec<[String; 4]>)> {
    prop::collection::vec((prop::array::uniform4(cell()), prop::array::uniform5(1..4usize)), 1..8).prop_map(|rows| {
        let mut text = String::from("# A runbook\nupdated: today\n\n| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n");
        let mut cells = Vec::new();
        for (i, (values, pads)) in rows.into_iter().enumerate() {
            let values: [String; 4] = std::array::from_fn(|k| clean(COLUMNS[k], &values[k]));
            let written: Vec<String> = values.iter().enumerate().map(|(k, v)| {
                let v = v.replace('|', "\\|");
                if COLUMNS[k] == Column::Command && !v.is_empty() { format!("`{v}`") } else { v }
            }).collect();
            let pad = |k: usize| " ".repeat(pads[k]);
            text.push('|');
            text.push_str(&format!("{}{}{}|", pad(0), i + 1, pad(0)));
            for (k, v) in written.iter().enumerate() {
                text.push_str(&format!("{}{v}{}|", pad(k + 1), pad(k + 1)));
            }
            text.push('\n');
            cells.push(values);
        }
        text.push_str("\n## Notes\n- a note with a | pipe\n");
        (text, cells)
    })
}

fn get(step: &Step, column: Column) -> &str {
    match column {
        Column::N => &step.n,
        Column::Step => &step.step,
        Column::Where => &step.where_,
        Column::Status => &step.status,
        Column::Command => &step.command,
    }
}

proptest! {
    #[test]
    fn parsing_never_panics(text in any::<String>()) {
        parse_text(Path::new("x.md"), &text);
    }

    #[test]
    fn parsing_never_panics_on_table_like_text(text in "[|\\\\` a#:\n-]{0,200}") {
        parse_text(Path::new("x.md"), &text);
    }

    #[test]
    fn writing_never_panics(text in "[|\\\\` a1#:\n-]{0,200}", n in "[0-9a]{1,2}", column in 0..5usize, value in any::<String>()) {
        let column = [Column::N, Column::Step, Column::Where, Column::Status, Column::Command][column];
        if let Some(out) = set_cell(&text, &n, column, &value) {
            parse_text(Path::new("x.md"), &out);
        }
        let (out, _) = insert_step(&text, Some(&n), &value, &value, &value);
        parse_text(Path::new("x.md"), &out);
    }

    #[test]
    fn a_written_cell_reads_back_and_nothing_else_moves((text, cells) in runbook(), row in any::<prop::sample::Index>(), column in 0..4usize, value in cell()) {
        let book = parse_text(Path::new("x.md"), &text);
        prop_assert_eq!(book.steps.len(), cells.len());
        for (step, values) in book.steps.iter().zip(&cells) {
            for (k, column) in COLUMNS.iter().enumerate() {
                prop_assert_eq!(get(step, *column), values[k].as_str());
            }
        }
        let row = row.index(cells.len());
        let column = COLUMNS[column];
        let out = set_cell(&text, &(row + 1).to_string(), column, &value).expect("the row is there");
        let after = parse_text(Path::new("x.md"), &out);
        prop_assert_eq!(after.steps.len(), book.steps.len());
        prop_assert_eq!(&after.notes, &book.notes);
        for (i, (was, now)) in book.steps.iter().zip(&after.steps).enumerate() {
            for other in [Column::N, Column::Step, Column::Where, Column::Status, Column::Command] {
                if i == row && other == column {
                    prop_assert_eq!(get(now, other), clean(column, &value));
                } else {
                    prop_assert_eq!(get(now, other), get(was, other));
                }
            }
        }
        // Only the row's own line changed.
        let changed: Vec<usize> = text.lines().zip(out.lines()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
        prop_assert!(changed.len() <= 1 && changed.iter().all(|&i| i == row + 5), "{changed:?}");
        prop_assert_eq!(text.lines().count(), out.lines().count());
    }

    #[test]
    fn an_inserted_step_renumbers_the_ones_after((text, cells) in runbook(), row in any::<prop::sample::Index>(), values in prop::array::uniform3(cell())) {
        let book = parse_text(Path::new("x.md"), &text);
        let row = row.index(cells.len());
        let (out, n) = insert_step(&text, Some(&(row + 1).to_string()), &values[0], &values[1], &values[2]);
        prop_assert_eq!(&n, &(row + 2).to_string());
        let after = parse_text(Path::new("x.md"), &out);
        prop_assert_eq!(after.steps.len(), book.steps.len() + 1);
        let new = &after.steps[row + 1];
        prop_assert_eq!(new.step.as_str(), clean(Column::Step, &values[0]));
        prop_assert_eq!(new.where_.as_str(), clean(Column::Where, &values[1]));
        prop_assert_eq!(new.status.as_str(), table::NEW_STATUS);
        prop_assert_eq!(new.command.as_str(), clean(Column::Command, &values[2]));
        for (i, step) in after.steps.iter().enumerate() {
            prop_assert_eq!(&step.n, &(i + 1).to_string());
            if i != row + 1 {
                let was = &book.steps[if i <= row { i } else { i - 1 }];
                prop_assert_eq!((&step.step, &step.where_, &step.status, &step.command), (&was.step, &was.where_, &was.status, &was.command));
            }
        }
    }
}
