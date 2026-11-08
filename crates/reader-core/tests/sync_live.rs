//! Two libraries syncing through a real sync service, with real books. Needs a running service:
//! `READER_API=https://reader-sync.tobygodat.workers.dev cargo test -p reader-core --test sync_live`.
//! Skips without it. Makes its own throwaway account and deletes it.

use std::path::PathBuf;

use reader_core::sync::{self, Client, RemoteSettings};
use reader_core::{Library, Position, prepare};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures").join(name)
}

fn no_settings() -> RemoteSettings {
    RemoteSettings { json: String::new(), updated_at: 0 }
}

fn sync_once(client: &Client, token: &str, lib: &mut Library, settings: &RemoteSettings) -> sync::Outcome {
    let outcome = sync::run(client, token, lib.dir(), lib.entries(), settings).expect("sync");
    lib.apply_remote(&outcome.books, None).expect("apply");
    outcome
}

#[test]
fn a_book_and_its_place_reach_the_other_device() {
    let Ok(api) = std::env::var("READER_API") else {
        eprintln!("READER_API isn't set: skipping");
        return;
    };
    let client = Client::new(api);
    let email = format!("live-{}@example.com", std::process::id());
    let session = client.sign_up(&email, "correct horse battery").expect("sign up");
    let token = session.token.as_str();

    let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a = Library::open(a_dir.path()).unwrap();
    let mut b = Library::open(b_dir.path()).unwrap();

    let prepared = prepare(a.dir(), &fixture("pride-and-prejudice.epub")).unwrap();
    let id = a.add(&prepared).unwrap().id.clone();
    a.set_position(&id, Position { chapter: 7, fraction: 0.25 }).unwrap();
    sync_once(&client, token, &mut a, &no_settings());

    sync_once(&client, token, &mut b, &no_settings());
    let entry = b.entry(&id).expect("the book reached the other device").clone();
    assert_eq!(entry.position, Position { chapter: 7, fraction: 0.25 });
    assert_eq!(b.load(&id).unwrap(), a.load(&id).unwrap(), "with its full text");
    let cover = b.cover_path(&entry).expect("and names its cover");
    assert_eq!(std::fs::read(cover).unwrap(), std::fs::read(a.cover_path(a.entry(&id).unwrap()).unwrap()).unwrap(), "and has it");

    b.set_position(&id, Position { chapter: 12, fraction: 0.5 }).unwrap();
    sync_once(&client, token, &mut b, &no_settings());
    sync_once(&client, token, &mut a, &no_settings());
    assert_eq!(a.entry(&id).unwrap().position, Position { chapter: 12, fraction: 0.5 }, "reading on the second device moves the first");

    let dark = RemoteSettings { json: r#"{"theme":"Night"}"#.into(), updated_at: 500 };
    sync_once(&client, token, &mut a, &dark);
    let out = sync_once(&client, token, &mut b, &no_settings());
    assert_eq!(out.settings, Some(dark), "settings travel too");

    let wrong = client.sign_in(&email, "not the password").unwrap_err();
    assert!(matches!(wrong, sync::SyncError::Refused(_)), "a wrong password is explained, not treated as signed out: {wrong:?}");
    client.delete_account(token).expect("delete the account");
    assert_eq!(sync::run(&client, token, b.dir(), b.entries(), &no_settings()).unwrap_err(), sync::SyncError::SignedOut);
}
