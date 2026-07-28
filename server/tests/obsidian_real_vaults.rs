//! Story 6.5 real-vault enumeration smoke test against Carver's actual Vaults.
//! Disabled by default (requires REAL_VAULTS env var) so CI / other machines
//! do not fail. Run locally with: REAL_VAULTS=1 cargo test --test obsidian_real_vaults
//!
//! Privacy: reads metadata only (enumerate_notes never opens note bodies).

use std::path::Path;

fn enabled() -> bool {
    std::env::var("REAL_VAULTS").map(|v| v == "1").unwrap_or(false)
}

#[test]
fn enumerate_real_vaults_matches_expected_counts() {
    if !enabled() {
        eprintln!("skipping real-vault test (set REAL_VAULTS=1 to run)");
        return;
    }
    for (name, root, min_expected) in [
        ("dev-repo", "/Users/carver/workspace/mindcarver/dev-repo", 1u64),
        ("91ai", "/Users/carver/workspace/mindcarver/91ai", 100u64),
    ] {
        let notes = tessera_lib::adapters::obsidian::enumerate_notes(Path::new(root))
            .unwrap_or_else(|e| panic!("enumerate {name} failed: {e}"));
        let count = notes.len() as u64;
        assert!(
            count >= min_expected,
            "{name}: expected >= {min_expected} notes, got {count}"
        );
        // Every note must have a forward-slash Vault-relative path, a size
        // within the 1 MiB bound, and be a .md file.
        for n in &notes {
            assert!(n.vault_relative_path.ends_with(".md"), "{:?}", n);
            assert!(!n.vault_relative_path.contains('\\'), "cross-platform forward-slash: {:?}", n);
            assert!(
                n.size <= tessera_lib::adapters::obsidian::MAX_NOTE_BYTES,
                "oversized note slipped through: {:?}",
                n
            );
        }
        println!("{name}: {count} notes enumerated (>= {min_expected})");
    }
}

#[test]
fn enumerate_real_vaults_excludes_obsidian_config_and_dotpaths() {
    if !enabled() {
        return;
    }
    for (_name, root) in [
        ("dev-repo", "/Users/carver/workspace/mindcarver/dev-repo"),
        ("91ai", "/Users/carver/workspace/mindcarver/91ai"),
    ] {
        let notes = tessera_lib::adapters::obsidian::enumerate_notes(Path::new(root))
            .expect("enumerate");
        for n in &notes {
            // No .obsidian, no .git, no dot-path should appear.
            assert!(
                !n.vault_relative_path.contains(".obsidian"),
                ".obsidian leaked: {:?}",
                n
            );
            assert!(
                !n.vault_relative_path.contains(".git"),
                ".git leaked: {:?}",
                n
            );
            let first = n.vault_relative_path.split('/').next().unwrap();
            assert!(
                !first.starts_with('.'),
                "dot-path leaked: {:?}",
                n
            );
        }
    }
}
