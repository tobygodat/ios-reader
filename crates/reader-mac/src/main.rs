fn main() {
    let data_dir = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("Library/Application Support/Reader");
    reader_app::configure(data_dir, reader_app::Host::Desktop);
    rune::run(reader_app::spec());
}
