//! Story 6.2 real-registry discovery smoke test. Gated on REAL_VAULTS so it
//! only runs on Carver's machine. Confirms the two known Vaults surface as
//! candidates from the real Obsidian registry.
#[test]
fn discover_real_registry_surfaces_known_vaults() {
    if std::env::var("REAL_VAULTS").map(|v| v != "1").unwrap_or(true) {
        eprintln!("skipping real-registry test (set REAL_VAULTS=1)");
        return;
    }
    let result = tessera_lib::adapters::obsidian::discover();
    let paths: Vec<&str> = result.candidates.iter().map(|c| c.root_path.as_str()).collect();
    println!("discovered {} vault candidates", result.candidates.len());
    if let Some(d) = &result.diagnostic {
        println!("diagnostic: {:?}", d);
    }
    for p in &paths {
        println!("  {}", p);
    }
    assert!(
        paths.iter().any(|p| p.ends_with("dev-repo")),
        "dev-repo vault must be discovered; got {:?}",
        paths
    );
    assert!(
        paths.iter().any(|p| p.ends_with("91ai")),
        "91ai vault must be discovered; got {:?}",
        paths
    );
    // No diagnostic expected on a healthy registry.
    assert!(
        result.diagnostic.is_none(),
        "healthy registry should not carry a diagnostic; got {:?}",
        result.diagnostic
    );
    // Every candidate is an obsidian/local_knowledge candidate.
    for c in &result.candidates {
        assert_eq!(c.provider, "obsidian");
    }
}
