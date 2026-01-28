use x3_runtime::{X3Executor, WasmMemory, host_functions::{InMemoryWasm, HostFunctionRegistry, HtlcEntry}};
use sha2::{Sha256, Digest};

#[test]
fn wasm_imported_htlc_claim_roundtrip() {
    // Build WASM that imports env.host_htlc_claim and exports it as 'claim'
    let mut wasm = vec![
        0x00, 0x61, 0x73, 0x6d,
        0x01, 0x00, 0x00, 0x00,
    ];

    // Type section: (i64,i64,i64) -> i64
    let mut type_sec = vec![];
    type_sec.push(1u8);
    type_sec.push(0x60);
    type_sec.push(3u8);
    type_sec.push(0x7e); type_sec.push(0x7e); type_sec.push(0x7e);
    type_sec.push(1u8); type_sec.push(0x7e);
    wasm.push(1u8);
    wasm.push(type_sec.len() as u8);
    wasm.extend_from_slice(&type_sec);

    // Import section
    let mut imp = vec![];
    imp.push(1u8);
    imp.push(3u8); imp.extend_from_slice(b"env");
    let name = b"host_htlc_claim"; imp.push(name.len() as u8); imp.extend_from_slice(name);
    imp.push(0u8); imp.push(0u8);
    wasm.push(2u8);
    wasm.push(imp.len() as u8);
    wasm.extend_from_slice(&imp);

    // Export section
    let mut exp = vec![];
    exp.push(1u8);
    exp.push(5u8); exp.extend_from_slice(b"claim");
    exp.push(0u8); exp.push(0u8);
    wasm.push(7u8); wasm.push(exp.len() as u8); wasm.extend_from_slice(&exp);

    // Prepare memory
    let mut mem = InMemoryWasm::new(2048);
    let preimage = b"integration-secret".to_vec();
    mem.write(128, &preimage).unwrap();
    let mut hasher = Sha256::new(); hasher.update(&preimage); let secret = hasher.finalize();

    let mut registry = HostFunctionRegistry::new().with_wasm_memory(std::sync::Arc::new(mem));
    if let Ok(mut m) = registry.htlc_store.write() {
        let mut sh = [0u8; 32]; sh.copy_from_slice(&secret[..32]);
        m.insert(42u64, HtlcEntry {
            id: 42,
            initiator: [0u8; 32], recipient: [0u8; 32], secret_hash: sh,
            amount: 100, timelock: 999, claimed: false, claimed_by: None,
        });
    }

    let executor = X3Executor::new().with_registry(registry);
    let caller = [7u8; 32];
    let result = executor.execute(&wasm, "claim", &[42u64, 128u64, preimage.len() as u64], caller);

    assert!(result.success, "executor error: {:?}", result.error);
}
