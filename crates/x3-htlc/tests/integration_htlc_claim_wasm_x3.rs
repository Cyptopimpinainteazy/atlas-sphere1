use x3_runtime::{X3Executor, WasmMemory, host_functions::{InMemoryWasm, HostFunctionRegistry, HtlcEntry}};
use sha2::{Sha256, Digest};

#[test]
fn wasm_x3_claim_flow() {
    // Build a WASM module where 'claim' is a local function that calls imported helpers:
    // imports: storage_read (i64,i64,i64)->i64, host_sha256 (i64,i64,i64)->i64, host_memcmp (i64,i64,i64)->i64, host_storage_mark_claimed (i64)->i64
    // We'll hand-craft a minimal module with one local function (type: (i64,i64,i64)->i64) that:
    // 1) calls storage_read(key_ptr,32, store_out_ptr)
    // 2) calls host_sha256(pre_ptr,pre_len, digest_ptr)
    // 3) calls host_memcmp(store_out_ptr, digest_ptr, 32) -> if equal -> host_storage_mark_claimed(id) -> return 1 else return 0

    let mut wasm = vec![
        0x00, 0x61, 0x73, 0x6d,
        0x01, 0x00, 0x00, 0x00,
    ];

    // Type section: two types: type0=(i64,i64,i64)->i64 (for imports 3-arg funcs and our claim); type1=(i64)->i64 (for mark_claimed)
    let mut types = vec![];
    // count = 2
    types.push(2u8);
    // type 0: func (params=3) -> (1)
    types.push(0x60); // func
    types.push(3u8); types.push(0x7e); types.push(0x7e); types.push(0x7e); // i64,i64,i64
    types.push(1u8); types.push(0x7e); // returns i64
    // type 1: func (params=1) -> (1)
    types.push(0x60);
    types.push(1u8); types.push(0x7e); // param i64
    types.push(1u8); types.push(0x7e); // returns i64

    wasm.push(1u8); // section id=1
    wasm.push(types.len() as u8);
    wasm.extend_from_slice(&types);

    // Import section: 4 imports
    let mut imp = vec![];
    imp.push(4u8); // count
    // For each import write: module_len + module + field_len + field + kind + typeidx
    // 0: storage_read
    imp.push(3u8); imp.extend_from_slice(b"env");
    let name = b"storage_read"; imp.push(name.len() as u8); imp.extend_from_slice(name);
    imp.push(0u8); // func
    imp.push(0u8); // type idx 0
    // 1: host_sha256
    imp.push(3u8); imp.extend_from_slice(b"env");
    let name = b"host_sha256"; imp.push(name.len() as u8); imp.extend_from_slice(name);
    imp.push(0u8); imp.push(0u8);
    // 2: host_memcmp
    imp.push(3u8); imp.extend_from_slice(b"env");
    let name = b"host_memcmp"; imp.push(name.len() as u8); imp.extend_from_slice(name);
    imp.push(0u8); imp.push(0u8);
    // 3: host_storage_mark_claimed (type idx 1)
    imp.push(3u8); imp.extend_from_slice(b"env");
    let name = b"host_storage_mark_claimed"; imp.push(name.len() as u8); imp.extend_from_slice(name);
    imp.push(0u8); imp.push(1u8);

    wasm.push(2u8); // import section id
    wasm.push(imp.len() as u8);
    wasm.extend_from_slice(&imp);

    // Function section: one local function, type idx 0
    let mut func_sec = vec![];
    func_sec.push(1u8); // count
    func_sec.push(0u8); // type idx 0
    wasm.push(3u8); wasm.push(func_sec.len() as u8); wasm.extend_from_slice(&func_sec);

    // Export section: export 'claim' -> func index = imports_count (4)
    let mut exp = vec![];
    exp.push(1u8);
    exp.push(5u8); exp.extend_from_slice(b"claim");
    exp.push(0u8); // kind func
    exp.push(4u8); // index 4 (after imports)
    wasm.push(7u8); wasm.push(exp.len() as u8); wasm.extend_from_slice(&exp);

    // Code section: one function body
    // We'll craft opcodes for the described logic.
    // Constants we'll use in module (explicit constants): key_ptr=64, key_len=32, store_out=256, digest_ptr=320
    let mut body = vec![];
    // local decls: 0 local entries
    body.push(0u8);
    // Instructions sequence:
    // i64.const 64
    body.push(0x42); body.push(64u8);
    // i64.const 32
    body.push(0x42); body.push(32u8);
    // i64.const 256 (LEB128 0x80 0x02)
    body.push(0x42); body.extend_from_slice(&[0x80u8, 0x02u8]);
    // call import 0 (storage_read)
    body.push(0x10); body.push(0u8);
    // drop result
    body.push(0x1a);

    // local.get 1 (pre_ptr)
    body.push(0x20); body.push(1u8);
    // local.get 2 (pre_len)
    body.push(0x20); body.push(2u8);
    // i64.const 320 (LEB128 0xC0 0x02)
    body.push(0x42); body.extend_from_slice(&[0xC0u8, 0x02u8]);
    // call import 1 (host_sha256)
    body.push(0x10); body.push(1u8);
    body.push(0x1a);

    // i64.const 256 (LEB128 0x80 0x02)
    body.push(0x42); body.extend_from_slice(&[0x80u8, 0x02u8]);
    // i64.const 320 (LEB128 0xC0 0x02)
    body.push(0x42); body.extend_from_slice(&[0xC0u8, 0x02u8]);
    // i64.const 32
    body.push(0x42); body.push(32u8);
    // call import 2 (host_memcmp)
    body.push(0x10); body.push(2u8);
    // i64.eqz (check equality)
    body.push(0x51);
    // if (result i64)
    body.push(0x04); body.push(0x7e);
      // local.get 0 (id)
      body.push(0x20); body.push(0u8);
      // call import 3 (host_storage_mark_claimed)
      body.push(0x10); body.push(3u8);
      body.push(0x1a); // drop
      // i64.const 1
      body.push(0x42); body.push(1u8);
    // else
    body.push(0x05);
      // i64.const 0
      body.push(0x42); body.push(0u8);
    // end
    body.push(0x0b);
    // end of function
    body.push(0x0b);

    // Wrap in body size LEB
    let mut code = vec![];
    code.push(1u8); // one function body
    // body size LEB - compute
    code.push(body.len() as u8);
    code.extend_from_slice(&body);

    wasm.push(10u8); wasm.push(code.len() as u8); wasm.extend_from_slice(&code);

    // For debugging: properly locate and dump import section bytes by parsing sections
    {
        let mut p = 8usize; // skip header
        while p + 1 < wasm.len() {
            let section_id = wasm[p]; p += 1;
            // read leb size
            let mut shift = 0usize; let mut size = 0usize; let mut br = 0usize;
            loop {
                let b = wasm[p + br];
                size |= ((b & 0x7f) as usize) << shift;
                br += 1;
                if b & 0x80 == 0 { break; }
                shift += 7;
            }
            let start = p + br;
            if section_id == 2 {
                eprintln!("import section bytes: {:?}", &wasm[start..start+size]);
                break;
            }
            p = start + size;
        }
    }

    // Prepare memory and registry
    let mut mem = InMemoryWasm::new(2048);
    // key at 64
    let id: u64 = 13;
    let mut key = [0u8; 32]; key[0] = 0xA0; key[1..9].copy_from_slice(&id.to_le_bytes());
    mem.write(64, &key).unwrap();

    // preimage at 128
    let preimage = b"wasm-claim-secret".to_vec();
    mem.write(128, &preimage).unwrap();
    let mut hasher = Sha256::new(); hasher.update(&preimage); let secret = hasher.finalize();

    // out buffer positions (store_out=256, digest=320) are left zero-filled

    let mut registry = HostFunctionRegistry::new().with_wasm_memory(std::sync::Arc::new(mem));

    // Populate kv_store with key -> secret
    if let Ok(mut kv) = registry.kv_store.write() {
        kv.insert(key, secret.to_vec());
    }

    // Also insert HTLC entry in htlc_store id=13
    if let Ok(mut m) = registry.htlc_store.write() {
        let mut sh = [0u8;32]; sh.copy_from_slice(&secret[..32]);
        m.insert(id, HtlcEntry{
            id,
            initiator: [0u8;32], recipient: [0u8;32], secret_hash: sh,
            amount: 123, timelock: 9999, claimed: false, claimed_by: None,
        });
    }

    // Keep a handle to the htlc store so we can inspect after moving registry into executor
    let htlc_store_clone = registry.htlc_store.clone();

    // Execute
    let executor = X3Executor::new().with_registry(registry);
    let caller = [5u8; 32];
    let result = executor.execute(&wasm, "claim", &[id as u64, 128u64, preimage.len() as u64], caller);

    assert!(result.success, "executor error: {:?}", result.error);

    // Confirm HTLC store shows claimed
    {
        let store = htlc_store_clone.read().unwrap();
        let e = store.get(&id).expect("htlc exists");
        assert!(e.claimed, "expected HTLC marked claimed");
    }
}
