use std::env;

mod binpack;
mod errors;
mod util;
mod wdl;

fn main() {
    let search_dir = env::args().nth(1).unwrap_or_else(|| {
        let current_dir = env::current_dir().expect("Failed to get current directory");
        current_dir.to_string_lossy().into_owned()
    });

    println!("Searching directory: {}", search_dir);

    // delete file if exists
    let output_path = std::path::Path::new("output.binpack");
    if output_path.exists() {
        std::fs::remove_file(output_path).expect("Failed to delete existing output.binpack");
    }

    let binpack_builder = binpack::BinpackBuilder::new(search_dir, "output.binpack");
    binpack_builder
        .create_binpack()
        .expect("Failed to create binpack");

    println!("Binpack created successfully.");

    let filesize = std::fs::metadata("output.binpack")
        .expect("Failed to get metadata")
        .len();

    println!(
        "Output file size: {} ",
        human_bytes::human_bytes(filesize as f64)
    );
}

// use sfbinpack::CompressedTrainingDataEntryReader;

// fn main() {
//     let mut reader = CompressedTrainingDataEntryReader::new("./output.binpack").unwrap();

//     let mut i = 0;

//     while reader.has_next() {
//         let entry = reader.next();

//         println!("entry:");
//         println!("fen {}", entry.pos.fen());
//         println!("uci move {:?}", entry.mv.as_uci());
//         println!("score {}", entry.score);
//         println!("ply {}", entry.ply);
//         println!("result {}", entry.result);
//         println!("\n");

//         i = i + 1;
//         if i > 200 {
//             break;
//         }
//     }
// }
