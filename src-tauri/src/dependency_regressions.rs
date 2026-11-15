#[test]
fn bundled_sqlite_is_patched_and_preserves_wal_transactions() {
    use rusqlite::Connection;

    // Check the library actually linked into the binary, including platform CI.
    assert!(rusqlite::version_number() >= 3_054_000, "RUSTSEC-2026-0459: {}", rusqlite::version());
    let temp_root = std::env::temp_dir();
    let base = temp_root.join(format!("sqlite-wal-regression-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&base).unwrap();
    let database = base.join("history.db");
    let writer = Connection::open(&database).unwrap();
    let mode: String = writer.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0)).unwrap();
    assert_eq!(mode, "wal");
    writer.execute_batch("PRAGMA wal_autocheckpoint=0; CREATE TABLE history(id INTEGER PRIMARY KEY, payload BLOB NOT NULL); INSERT INTO history VALUES(1, X'001122ff');").unwrap();
    assert!(std::fs::metadata(base.join("history.db-wal")).unwrap().len() > 32);

    let reader = Connection::open(&database).unwrap();
    let payload = || reader.query_row("SELECT payload FROM history WHERE id=1", [], |row| row.get::<_, Vec<u8>>(0)).unwrap();
    assert_eq!(payload(), [0, 0x11, 0x22, 0xff]);
    writer.execute_batch("BEGIN IMMEDIATE; UPDATE history SET payload=X'99' WHERE id=1;").unwrap();
    assert_eq!(payload(), [0, 0x11, 0x22, 0xff], "uncommitted data must stay isolated");
    writer.execute_batch("ROLLBACK; BEGIN IMMEDIATE; INSERT INTO history VALUES(2, X'aabb'); COMMIT;").unwrap();
    assert_eq!(payload(), [0, 0x11, 0x22, 0xff]);
    let count: i64 = reader.query_row("SELECT count(*) FROM history", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 2);
    drop(reader);
    writer.close().unwrap();

    let reopened = Connection::open(&database).unwrap();
    let rows: Vec<Vec<u8>> = reopened.prepare("SELECT payload FROM history ORDER BY id").unwrap()
        .query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(rows, vec![vec![0, 0x11, 0x22, 0xff], vec![0xaa, 0xbb]]);
    assert_eq!(reopened.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)).unwrap(), "ok");
    reopened.close().unwrap();
    assert_eq!(base.parent(), Some(temp_root.as_path()));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn urlpattern_identifier_backport_preserves_url_boundaries() {
    use urlpattern::{UrlPattern, UrlPatternInit, UrlPatternMatchInput};
    for name in ["id", "编号", "Δvalue", "_value", "$value"] {
        let pattern = <UrlPattern>::parse(
            UrlPatternInit {
                protocol: Some("https".into()),
                hostname: Some("example.test".into()),
                pathname: Some(format!("/users/:{name}")),
                ..Default::default()
            },
            Default::default(),
        )
        .unwrap();
        let matched = pattern
            .exec(UrlPatternMatchInput::Url(
                "https://example.test/users/123".parse().unwrap(),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(matched.pathname.groups[name].as_deref(), Some("123"));
        assert!(!pattern
            .test(UrlPatternMatchInput::Url(
                "https://other.test/users/123".parse().unwrap()
            ))
            .unwrap());
    }
}

#[test]
fn macro_backports_preserve_valid_expansions_and_report_invalid_inputs() {
    use std::{fs, process::Command};
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let mut externs = Vec::new();
    for name in ["glib_macros", "gtk3_macros"] {
        let mut libraries: Vec<_> = fs::read_dir(&deps)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                let filename = entry.file_name().to_string_lossy().into_owned();
                (filename.starts_with(&format!("{name}-"))
                    || filename.starts_with(&format!("lib{name}-")))
                    && ["dll", "so", "dylib"]
                        .iter()
                        .any(|ext| filename.ends_with(&format!(".{ext}")))
            })
            .collect();
        libraries.sort_by_key(|entry| entry.metadata().unwrap().modified().unwrap());
        externs.push(format!(
            "{name}={}",
            libraries
                .last()
                .expect("macro library was built")
                .path()
                .display()
        ));
    }
    let base = std::env::temp_dir().join(format!("macro-regression-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("Cargo.toml"), "[package]\nname=\"fixture\"\nversion=\"0.0.0\"\n[dependencies]\nglib=\"0.18\"\ngtk=\"0.18\"\n").unwrap();
    let source = base.join("fixture.rs");
    let fixtures = [
        ("const _: &[u8] = glib_macros::cstr_bytes!(\"hello\"); fn valid() { let value = 1; let _ = glib_macros::clone!(@strong value => move || value); }", None),
        ("#[derive(glib_macros::Enum)] enum Invalid { A }", Some("requires #[enum_type")),
        ("fn main() { glib_macros::closure!(async || {}); }", Some("Async closure not allowed")),
        ("#[derive(gtk3_macros::CompositeTemplate)] struct Invalid;", Some("requires #[template")),
    ];
    for (text, expected) in fixtures {
        fs::write(&source, text).unwrap();
        let mut command = Command::new("rustc");
        command
            .env("CARGO_MANIFEST_DIR", &base)
            .env("CARGO_PKG_NAME", "fixture");
        command
            .args([
                "--edition=2021",
                "--crate-type=lib",
                "--emit=metadata",
                "--out-dir",
            ])
            .arg(&base)
            .arg("-L")
            .arg(format!("dependency={}", deps.display()))
            .arg(&source);
        for item in &externs {
            command.arg("--extern").arg(item);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command.output().unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        if let Some(expected) = expected {
            assert!(!output.status.success());
            assert!(errors.contains(expected), "{errors}");
            assert!(!errors.contains("proc macro panicked"), "{errors}");
        } else {
            assert!(output.status.success(), "{errors}");
        }
    }
    fs::remove_dir_all(base).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn optimized_glib_string_iterator_backport_covers_both_directions() {
    use glib::variant::ToVariant;
    let value = ["alpha", "beta", "gamma"].to_variant();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some("alpha"));
    assert_eq!(iter.next_back(), Some("gamma"));
    assert_eq!(iter.next(), Some("beta"));
    assert_eq!(iter.next(), None);
    assert_eq!(value.array_iter_str().unwrap().nth(1), Some("beta"));
    assert_eq!(value.array_iter_str().unwrap().nth_back(1), Some("beta"));
    assert_eq!(value.array_iter_str().unwrap().last(), Some("gamma"));
}
