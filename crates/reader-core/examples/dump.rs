//! `cargo run -p reader-core --example dump -- <file> [chapter]` prints what a file parses into.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = std::path::Path::new(&args[1]);
    let bytes = std::fs::read(path).expect("readable file");
    let book = match reader_core::import::parse(path, &bytes) {
        Ok(b) => b,
        Err(e) => return eprintln!("{e}"),
    };
    println!("{} / {:?} / {} chapters", book.title, book.author, book.chapters.len());
    match args.get(2).and_then(|c| c.parse::<usize>().ok()) {
        Some(i) => {
            let c = &book.chapters[i];
            println!("== {:?}", c.title);
            for b in &c.blocks {
                println!("{b:?}");
            }
        }
        None => {
            for (i, c) in book.chapters.iter().enumerate() {
                println!("{i:3} {:?} ({} blocks) {:.70?}", c.title, c.blocks.len(), c.blocks.first());
            }
        }
    }
}
