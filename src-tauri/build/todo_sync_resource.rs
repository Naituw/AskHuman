//! Resource generation shared by build.rs and a standalone, account-free release regression test.

use std::io;
use std::path::Path;

pub fn write(out: &Path, target_os: &str, source: Option<&Path>, required: bool) -> io::Result<()> {
    let declaration = if target_os == "macos" {
        match source {
            Some(source) => {
                std::fs::copy(source, out.join("todo-sync-helper.zip"))?;
                "pub static BUNDLED_HELPER: Option<&[u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/todo-sync-helper.zip\")));"
            }
            None if required => {
                return Err(io::Error::other(
                    "public macOS releases require ASKHUMAN_TODO_SYNC_ARCHIVE",
                ))
            }
            None => "pub static BUNDLED_HELPER: Option<&[u8]> = None;",
        }
    } else {
        "pub static BUNDLED_HELPER: Option<&[u8]> = None;"
    };
    std::fs::write(out.join("todo-sync-resource.rs"), declaration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "askhuman-resource-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_required_resource_stops_a_macos_release() {
        let out = Temp::new();
        assert!(write(&out.0, "macos", None, true).is_err());
        assert!(!out.0.join("todo-sync-resource.rs").exists());
    }

    #[test]
    fn an_unreadable_resource_is_not_silently_omitted() {
        let out = Temp::new();
        assert!(write(&out.0, "macos", Some(&out.0.join("missing.zip")), true).is_err());
    }

    #[test]
    fn other_platforms_never_embed_an_accidentally_supplied_resource() {
        for os in ["linux", "windows"] {
            let out = Temp::new();
            write(&out.0, os, Some(&out.0.join("missing.zip")), true).unwrap();
            assert!(!out.0.join("todo-sync-helper.zip").exists());
            assert!(std::fs::read_to_string(out.0.join("todo-sync-resource.rs"))
                .unwrap()
                .contains("= None;"));
        }
    }

    #[test]
    fn source_builds_can_omit_the_resource() {
        let out = Temp::new();
        write(&out.0, "macos", None, false).unwrap();
        assert!(!out.0.join("todo-sync-helper.zip").exists());
    }

    #[test]
    fn the_compiled_executable_carries_the_exact_archive_bytes() {
        let out = Temp::new();
        let payload = b"PK\x03\x04a binary resource\x00\xff";
        let source = out.0.join("source.zip");
        std::fs::write(&source, payload).unwrap();
        write(&out.0, "macos", Some(&source), true).unwrap();
        let consumer = out.0.join("consumer.rs");
        std::fs::write(&consumer, "include!(concat!(env!(\"OUT_DIR\"), \"/todo-sync-resource.rs\")); fn main() { use std::io::Write; std::io::stdout().write_all(BUNDLED_HELPER.unwrap()).unwrap(); }").unwrap();
        let executable = out.0.join(if cfg!(windows) {
            "consumer.exe"
        } else {
            "consumer"
        });
        let compiled = std::process::Command::new("rustc")
            .arg(&consumer)
            .arg("-o")
            .arg(&executable)
            .env("OUT_DIR", &out.0)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let output = std::process::Command::new(executable).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, payload);
    }
}
