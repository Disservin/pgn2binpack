use std::fs::File;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use sfbinpack::chess::piece::Piece;
use sfbinpack::CompressedTrainingDataEntryReader;

const VALUE_NONE_SCORE: i16 = 32002;

#[derive(Parser, Debug)]
#[command(about = "Count zero scores after large prior scores in sfbinpack files")]
struct Cli {
    /// Input sfbinpack file
    #[arg(value_name = "FILE")]
    input: PathBuf,

    /// Only process the first N entries
    #[arg(long)]
    limit: Option<u128>,

    /// Absolute threshold for the previous known score
    #[arg(long, default_value_t = 100)]
    threshold: i16,
}

fn format_num(n: u128) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn is_capturing_move(entry: &sfbinpack::TrainingDataEntry) -> bool {
    entry.pos.piece_at(entry.mv.to()) != Piece::none()
        && entry.pos.piece_at(entry.mv.to()).color() != entry.pos.piece_at(entry.mv.from()).color()
}

fn in_check(entry: &sfbinpack::TrainingDataEntry) -> bool {
    entry.pos.is_checked(entry.pos.side_to_move())
}

fn skipped_reason(entry: &sfbinpack::TrainingDataEntry) -> Option<&'static str> {
    if entry.score != 0 {
        return None;
    }

    match (is_capturing_move(entry), in_check(entry)) {
        (true, true) => Some("capture + in-check"),
        (true, false) => Some("capture"),
        (false, true) => Some("in-check"),
        (false, false) => None,
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let file = File::open(&cli.input)?;
    let mut reader = CompressedTrainingDataEntryReader::new(file)?;

    let mut processed = 0u128;
    let mut zero_total_raw = 0u128;
    let mut zero_skipped = 0u128;
    let mut zeros_after_large = 0u128;
    let mut zero_with_known_previous = 0u128;
    let mut zero_total = 0u128;
    let mut last_known_score: Option<i16> = None;

    while reader.has_next() {
        let entry = reader.next();
        let score = entry.score;

        if score == 0 {
            zero_total_raw += 1;

            if skipped_reason(&entry).is_some() {
                zero_skipped += 1;
            } else {
                zero_total += 1;

                if let Some(previous) = last_known_score {
                    zero_with_known_previous += 1;
                    if previous.abs() > cli.threshold {
                        zeros_after_large += 1;
                    }
                }
            }
        }

        if score != VALUE_NONE_SCORE {
            last_known_score = Some(score);
        }

        processed += 1;
        if let Some(limit) = cli.limit {
            if processed >= limit {
                break;
            }
        }

        if !(reader.has_next() && reader.is_next_entry_continuation()) {
            last_known_score = None;
        }
    }

    println!("file: {}", cli.input.display());
    println!(
        "entries processed:               {:>12}",
        format_num(processed)
    );
    println!(
        "zero scores (raw total):         {:>12}",
        format_num(zero_total_raw)
    );
    if zero_total_raw > 0 {
        println!(
            "  └─ recorded as skipped:        {:>12}   ({:6.3}% of raw zero scores)",
            format_num(zero_skipped),
            (zero_skipped as f64 / zero_total_raw as f64) * 100.0
        );
    }
    println!(
        "zero scores (non-skipped):       {:>12}",
        format_num(zero_total)
    );

    if zero_total > 0 {
        println!(
            "  └─ with known previous score:  {:>12}   ({:6.3}% of non-skipped zeros)",
            format_num(zero_with_known_previous),
            (zero_with_known_previous as f64 / zero_total as f64) * 100.0
        );
    }

    if zero_with_known_previous > 0 {
        println!(
            "  └─ after |prev score| > {:>4}:  {:>12}   ({:6.3}% of zero w/ known prev)",
            cli.threshold,
            format_num(zeros_after_large),
            (zeros_after_large as f64 / zero_with_known_previous as f64) * 100.0
        );
        println!(
            "                                                ({:6.3}% of all entries)",
            (zeros_after_large as f64 / processed as f64) * 100.0
        );
    }

    Ok(())
}
