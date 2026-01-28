// crates/x3-runtime/src/host_functions.rs
// Host function implementations for X3 extern declarations
//
// These implement the host ABI defined in stdlib files:
// - core.x3: panic, require, assert
// - bridge.x3: host_send_message, host_verify_proof, etc.
// - token.x3: evm_*, svm_* token operations
// - dex.x3: host_quote_swap, host_swap_exact_in, etc.

use crate::{X3Context, BridgeMessage, X3StateChange};
use anyhow::{anyhow, Result};
use parity_scale_codec::{Encode, Decode};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Trait for reading WASM linear memory
/// Implemented by the WASM runtime to provide memory access to host functions
pub trait WasmMemory: Send + Sync {
    /// Read bytes from WASM memory at the given offset
    fn read(&self, offset: u32, length: u32) -> Result<Vec<u8>>;
    
    /// Write bytes to WASM memory at the given offset
    fn write(&self, offset: u32, data: &[u8]) -> Result<()>;
    
    /// Get the current memory size in bytes
    fn size(&self) -> u32;
}

/// Default in-memory implementation for testing
pub struct InMemoryWasm {
    data: RwLock<Vec<u8>>,
}

impl InMemoryWasm {
    pub fn new(size: usize) -> Self {
        InMemoryWasm {
            data: RwLock::new(vec![0u8; size]),
        }
    }
    
    pub fn with_data(data: Vec<u8>) -> Self {
        InMemoryWasm {
            data: RwLock::new(data),
        }
    }
}

impl WasmMemory for InMemoryWasm {
    fn read(&self, offset: u32, length: u32) -> Result<Vec<u8>> {
        let data = self.data.read().map_err(|_| anyhow!("Memory lock poisoned"))?;
        let start = offset as usize;
        let end = start + length as usize;
        if end > data.len() {
            return Err(anyhow!("Memory read out of bounds: {} + {} > {}", offset, length, data.len()));
        }
        Ok(data[start..end].to_vec())
    }
    
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<()> {
        let mut data = self.data.write().map_err(|_| anyhow!("Memory lock poisoned"))?;
        let start = offset as usize;
        let end = start + bytes.len();
        if end > data.len() {
            return Err(anyhow!("Memory write out of bounds: {} + {} > {}", offset, bytes.len(), data.len()));
        }
        data[start..end].copy_from_slice(bytes);
        Ok(())
    }
    
    fn size(&self) -> u32 {
        self.data.read().map(|d| d.len() as u32).unwrap_or(0)
    }
}

/// Host function registry
/// Maps function names to their implementations
pub struct HostFunctionRegistry {
    functions: HashMap<String, HostFn>,
    /// Token balances: (token_id, account) -> balance
    token_balances: Arc<RwLock<HashMap<([u8; 32], [u8; 32]), u128>>>,
    /// Token allowances: (token_id, owner, spender) -> allowance
    token_allowances: Arc<RwLock<HashMap<([u8; 32], [u8; 32], [u8; 32]), u128>>>,
    /// ZK proof verifier
    proof_verifier: Arc<dyn ProofVerifier + Send + Sync>,
    /// WASM memory for reading/writing guest data
    wasm_memory: Option<Arc<dyn WasmMemory>>,
    /// Bridge receipts: msg_id -> (success, return_data)
    bridge_receipts: Arc<RwLock<HashMap<[u8; 32], (bool, Vec<u8>)>>>,
    /// Bridge state: msg_id -> state (0=Pending, 1=Executed, 2=Failed, 3=Finalized)
    bridge_states: Arc<RwLock<HashMap<[u8; 32], u8>>>,
    /// Canonical bridge roots committed
    bridge_roots: Arc<RwLock<Vec<[u8; 32]>>>,
    /// HTLC store: id -> HTLC entry
    pub htlc_store: Arc<RwLock<HashMap<u64, HtlcEntry>>>,
    /// Simple in-memory KV store to simulate contract storage for tests
    pub kv_store: Arc<RwLock<HashMap<[u8; 32], Vec<u8>>>>,
}

/// In-memory HTLC entry used by host functions (test/runtime simulation)
#[derive(Clone, Debug)]
pub struct HtlcEntry {
    pub id: u64,
    pub initiator: [u8; 32],
    pub recipient: [u8; 32],
    pub secret_hash: [u8; 32],
    pub amount: u128,
    pub timelock: u64,
    pub claimed: bool,
    pub claimed_by: Option<[u8; 32]>,
}

/// Host function signature - use boxed dyn so closures capturing env can be stored
pub type HostFn = Box<dyn Fn(&mut X3Context, &[u64]) -> Result<u64> + Send + Sync>;

/// Proof verifier trait
pub trait ProofVerifier: Send + Sync {
    fn verify(&self, proof: &[u8]) -> bool;
}

/// Mock proof verifier for testing
pub struct MockProofVerifier;

impl ProofVerifier for MockProofVerifier {
    fn verify(&self, proof: &[u8]) -> bool {
        // Accept proofs that are non-empty and start with 0x01
        !proof.is_empty() && proof[0] == 0x01
    }
}

impl HostFunctionRegistry {
    pub fn new() -> Self {
        let mut registry = HostFunctionRegistry {
            functions: HashMap::new(),
            token_balances: Arc::new(RwLock::new(HashMap::new())),
            token_allowances: Arc::new(RwLock::new(HashMap::new())),
            proof_verifier: Arc::new(MockProofVerifier),
            wasm_memory: None,
            bridge_receipts: Arc::new(RwLock::new(HashMap::new())),
            bridge_states: Arc::new(RwLock::new(HashMap::new())),
            bridge_roots: Arc::new(RwLock::new(Vec::new())),
            htlc_store: Arc::new(RwLock::new(HashMap::new())),
            kv_store: Arc::new(RwLock::new(HashMap::new())),
        };
        
        registry.register_core_functions();
        registry.register_bridge_functions();
        
        registry
    }

    pub fn with_proof_verifier(verifier: Arc<dyn ProofVerifier + Send + Sync>) -> Self {
        let mut registry = Self::new();
        registry.proof_verifier = verifier;
        registry
    }
    
    pub fn with_wasm_memory(mut self, memory: Arc<dyn WasmMemory>) -> Self {
        self.wasm_memory = Some(memory);
        // Re-register to ensure closures capture the configured memory
        self.register_core_functions();
        self.register_bridge_functions();
        self
    }
    
    /// Read bytes from WASM memory if available
    pub fn read_wasm_memory(&self, ptr: u32, len: u32) -> Result<Vec<u8>> {
        match &self.wasm_memory {
            Some(mem) => mem.read(ptr, len),
            None => Err(anyhow!("WASM memory not available")),
        }
    }
    
    /// Write bytes to WASM memory if available
    pub fn write_wasm_memory(&self, ptr: u32, data: &[u8]) -> Result<()> {
        match &self.wasm_memory {
            Some(mem) => mem.write(ptr, data),
            None => Err(anyhow!("WASM memory not available")),
        }
    }
    
    /// Store a bridge receipt
    pub fn store_bridge_receipt(&self, msg_id: [u8; 32], success: bool, data: Vec<u8>) {
        if let Ok(mut receipts) = self.bridge_receipts.write() {
            receipts.insert(msg_id, (success, data));
        }
        if let Ok(mut states) = self.bridge_states.write() {
            states.insert(msg_id, if success { 1 } else { 2 }); // Executed or Failed
        }
    }
    
    /// Finalize a bridge message
    pub fn finalize_bridge_message(&self, msg_id: [u8; 32]) {
        if let Ok(mut states) = self.bridge_states.write() {
            states.insert(msg_id, 3); // Finalized
        }
    }

    fn register_core_functions(&mut self) {
        // Gas cost for core operations
        const CORE_GAS: u64 = 100;

        self.functions.insert("host_get_chain_id".into(), Box::new(|ctx, _args| {
            ctx.consume_gas(CORE_GAS)?;
            Ok(ctx.chain_id as u64)
        }));

        self.functions.insert("host_get_block_height".into(), Box::new(|ctx, _args| {
            ctx.consume_gas(CORE_GAS)?;
            Ok(ctx.block_height)
        }));

        self.functions.insert("caller_address".into(), Box::new(|ctx, _args| {
            ctx.consume_gas(CORE_GAS)?;
            // Return first 8 bytes as u64 (simplified for MVP)
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&ctx.caller[0..8]);
            Ok(u64::from_le_bytes(bytes))
        }));

        self.functions.insert("self_address".into(), Box::new(|ctx, _args| {
            ctx.consume_gas(CORE_GAS)?;
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&ctx.self_addr[0..8]);
            Ok(u64::from_le_bytes(bytes))
        }));

        // host_sha256(ptr: u64, len: u64, out_ptr: u64) -> i64 (1=ok,0=fail)
        let wasm_mem = self.wasm_memory.clone();
        self.functions.insert("host_sha256".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(CORE_GAS * 10)?; // base cost for hashing

            if args.len() < 3 {
                return Err(anyhow!("host_sha256 requires 3 args: ptr, len, out_ptr"));
            }

            let ptr = args[0] as u32;
            let len = args[1] as u32;
            let out_ptr = args[2] as u32;

            // Read input from WASM memory
            let mem = match &wasm_mem {
                Some(m) => m,
                None => return Err(anyhow!("WASM memory not configured for host_sha256")),
            };

            if len == 0 || len > 65536 {
                return Ok(0);
            }

            let data = mem.read(ptr, len)?;

            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let digest = hasher.finalize();

            // Write digest (32 bytes) back to WASM memory at out_ptr
            mem.write(out_ptr, &digest)?;

            Ok(1)
        }));
    }

    fn register_bridge_functions(&mut self) {
        const BRIDGE_GAS: u64 = 5000;

        // Clone Arc references for use in closures
        let bridge_receipts = Arc::clone(&self.bridge_receipts);
        let bridge_states = Arc::clone(&self.bridge_states);
        let bridge_roots = Arc::clone(&self.bridge_roots);

self.functions.insert("host_send_message".into(), Box::new(|ctx, args| {
            ctx.consume_gas(BRIDGE_GAS)?;
            
            if args.len() < 4 {
                return Err(anyhow!("host_send_message requires 4 args: dst_chain, payload_ptr, payload_len, gas_limit"));
            }

            let dst_chain = args[0] as u32;
            let payload_ptr = args[1] as u32;
            let payload_len = args[2] as u32;
            let gas_limit = args[3];

            // Read payload from context's pending memory buffer if available
            // In production, this reads from WASM linear memory via the executor
            let payload = if payload_len > 0 && payload_len <= 16384 {
                // For now, create a placeholder payload with the pointer info
                // Real implementation would use: registry.read_wasm_memory(payload_ptr, payload_len)?
                let mut p = Vec::with_capacity(payload_len as usize);
                p.extend_from_slice(&payload_ptr.to_le_bytes());
                p.extend_from_slice(&payload_len.to_le_bytes());
                p.resize(payload_len as usize, 0);
                p
            } else {
                Vec::new()
            };

            let msg = BridgeMessage {
                src_chain: ctx.chain_id,
                dst_chain,
                sender: ctx.caller,
                payload,
                gas_limit,
                nonce: ctx.bridge_messages.len() as u64,
            };
            
            let msg_id = ctx.send_bridge_message(msg);
            
            // Return first 8 bytes of msg_id as u64
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&msg_id[0..8]);
            Ok(u64::from_le_bytes(bytes))
        }));

        // Register HTLC host functions
        let htlc_store = Arc::clone(&self.htlc_store);
        let wasm_mem_clone = self.wasm_memory.clone();
        self.functions.insert("host_htlc_create".into(), Box::new(move |_ctx, args| {
            // args: id, secret_ptr, secret_len, amount_low, timelock
            if args.len() < 5 {
                return Err(anyhow!("host_htlc_create requires args: id, secret_ptr, secret_len, amount_lo, timelock"));
            }
            let id = args[0];
            let secret_ptr = args[1] as u32;
            let secret_len = args[2] as u32;
            let amount = args[3];
            let timelock = args[4];

            let mem = match &wasm_mem_clone {
                Some(m) => m,
                None => return Err(anyhow!("WASM memory not available for host_htlc_create")),
            };

            if secret_len != 32 {
                return Err(anyhow!("secret_len must be 32"));
            }

            let secret = mem.read(secret_ptr, secret_len)?;
            let mut sh = [0u8; 32];
            sh.copy_from_slice(&secret[..32]);

            let entry = HtlcEntry {
                id: id as u64,
                initiator: [0u8; 32],
                recipient: [0u8; 32],
                secret_hash: sh,
                amount: amount as u128,
                timelock: timelock as u64,
                claimed: false,
                claimed_by: None,
            };

            if let Ok(mut m) = htlc_store.write() {
                m.insert(id as u64, entry);
            }

            Ok(1)
        }));

        let htlc_store2 = Arc::clone(&self.htlc_store);
        let wasm_mem_clone2 = self.wasm_memory.clone();
        self.functions.insert("host_htlc_claim".into(), Box::new(move |ctx, args| {
            // args: id, preimage_ptr, preimage_len
            ctx.consume_gas(BRIDGE_GAS)?; // reuse bridge gas for now

            if args.len() < 3 {
                return Err(anyhow!("host_htlc_claim requires args: id, preimage_ptr, preimage_len"));
            }

            let id = args[0] as u64;
            let pre_ptr = args[1] as u32;
            let pre_len = args[2] as u32;

            let mem = match &wasm_mem_clone2 {
                Some(m) => m,
                None => return Err(anyhow!("WASM memory not available for host_htlc_claim")),
            };

            if pre_len == 0 || pre_len > 1024 {
                return Ok(0);
            }

            let pre = mem.read(pre_ptr, pre_len)?;

            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(&pre);
            let digest = hasher.finalize();

            if let Ok(mut m) = htlc_store2.write() {
                if let Some(entry) = m.get_mut(&id) {
                    if entry.claimed {
                        return Ok(2); // already claimed
                    }
                    if entry.secret_hash[..] == digest[..] {
                        entry.claimed = true;
                        entry.claimed_by = Some(ctx.caller);
                        // Emit log
                        ctx.emit_log("HTLCClaimed".into(), id.to_le_bytes().to_vec());
                        return Ok(1);
                    } else {
                        return Ok(3); // invalid preimage
                    }
                }
            }

            Ok(0)
        }));

        // storage_read(key_ptr:u64, key_len:u64, out_ptr:u64) -> len_written (u64) or 0
        let kv_store_read = Arc::clone(&self.kv_store);
        let wasm_mem_for_read = self.wasm_memory.clone();
        self.functions.insert("storage_read".into(), Box::new(move |_ctx, args| {
            if args.len() < 3 {
                return Err(anyhow!("storage_read requires args: key_ptr, key_len, out_ptr"));
            }
            let key_ptr = args[0] as u32;
            let key_len = args[1] as u32;
            let out_ptr = args[2] as u32;

            let mem = match &wasm_mem_for_read {
                Some(m) => m,
                None => return Err(anyhow!("WASM memory not available for storage_read")),
            };

            if key_len != 32 {
                return Ok(0);
            }

            let key_bytes = mem.read(key_ptr, key_len)?;
            let mut key = [0u8; 32]; key.copy_from_slice(&key_bytes[..32]);

            if let Ok(store) = kv_store_read.read() {
                if let Some(val) = store.get(&key) {
                    if val.len() > 1024 {
                        return Err(anyhow!("Value too large"));
                    }
                    mem.write(out_ptr, val)?;
                    return Ok(val.len() as u64);
                }
            }
            Ok(0)
        }));

        // host_memcmp(a_ptr:u64, b_ptr:u64, len:u64) -> 0 if equal, 1 otherwise
        let wasm_mem_for_memcmp = self.wasm_memory.clone();
        self.functions.insert("host_memcmp".into(), Box::new(move |_ctx, args| {
            if args.len() < 3 {
                return Err(anyhow!("host_memcmp requires args: a_ptr, b_ptr, len"));
            }
            let a_ptr = args[0] as u32;
            let b_ptr = args[1] as u32;
            let len = args[2] as u32;

            let mem = match &wasm_mem_for_memcmp {
                Some(m) => m,
                None => return Err(anyhow!("WASM memory not available for host_memcmp")),
            };

            if len == 0 || len > 65536 {
                return Ok(1);
            }

            let a = mem.read(a_ptr, len)?;
            let b = mem.read(b_ptr, len)?;
            if a == b { Ok(0) } else { Ok(1) }
        }));

        // host_storage_mark_claimed(id:u64) -> 1 on success, 2 already claimed, 0 not found
        let htlc_store_mark = Arc::clone(&self.htlc_store);
        self.functions.insert("host_storage_mark_claimed".into(), Box::new(move |ctx, args| {
            if args.is_empty() {
                return Err(anyhow!("host_storage_mark_claimed requires id arg"));
            }
            let id = args[0] as u64;
            if let Ok(mut m) = htlc_store_mark.write() {
                if let Some(entry) = m.get_mut(&id) {
                    if entry.claimed {
                        return Ok(2);
                    }
                    entry.claimed = true;
                    entry.claimed_by = Some(ctx.caller);
                    ctx.emit_log("HTLCClaimedByWASM".into(), id.to_le_bytes().to_vec());
                    return Ok(1);
                }
            }
            Ok(0)
        }));

        let roots_clone = Arc::clone(&bridge_roots);
        self.functions.insert("host_commit_bridge_root".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(BRIDGE_GAS)?;
            
            if args.is_empty() {
                return Err(anyhow!("host_commit_bridge_root requires root arg"));
            }
            
            // Convert the u64 arg to a 32-byte root
            let root_low = args[0];
            let root_high = if args.len() > 1 { args[1] } else { 0 };
            
            let mut root = [0u8; 32];
            root[0..8].copy_from_slice(&root_low.to_le_bytes());
            root[8..16].copy_from_slice(&root_high.to_le_bytes());
            
            // Commit root to bridge roots storage
            if let Ok(mut roots) = roots_clone.write() {
                roots.push(root);
            }
            
            // Emit log for the commitment
            ctx.emit_log(
                "BridgeRootCommitted".into(),
                root.to_vec(),
            );
            
            Ok(1) // Success
        }));

        let receipts_clone = Arc::clone(&bridge_receipts);
        self.functions.insert("host_resolve_bridge_receipt".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(BRIDGE_GAS)?;
            
            if args.is_empty() {
                return Err(anyhow!("host_resolve_bridge_receipt requires msg_id arg"));
            }
            
            // Convert u64 arg to msg_id lookup key
            let msg_id_low = args[0];
            let mut msg_id = [0u8; 32];
            msg_id[0..8].copy_from_slice(&msg_id_low.to_le_bytes());
            
            // Look up receipt by msg_id
            if let Ok(receipts) = receipts_clone.read() {
                if let Some((success, data)) = receipts.get(&msg_id) {
                    // Return encoded receipt info
                    // Format: success (1 byte) + data_len (4 bytes) + data_hash_first_3_bytes
                    let success_byte = if *success { 1u64 } else { 0u64 };
                    let data_len = data.len() as u64;
                    return Ok(success_byte | (data_len << 8));
                }
            }
            
            Ok(0) // None - no receipt found
        }));

        let states_clone = Arc::clone(&bridge_states);
        self.functions.insert("host_get_bridge_state".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(BRIDGE_GAS / 2)?;
            
            if args.is_empty() {
                return Err(anyhow!("host_get_bridge_state requires msg_id arg"));
            }
            
            // Convert u64 arg to msg_id lookup key
            let msg_id_low = args[0];
            let mut msg_id = [0u8; 32];
            msg_id[0..8].copy_from_slice(&msg_id_low.to_le_bytes());
            
            // Look up state by msg_id
            if let Ok(states) = states_clone.read() {
                if let Some(&state) = states.get(&msg_id) {
                    return Ok(state as u64);
                }
            }
            
            // Return BridgeState enum as u64
            // 0 = Pending, 1 = Executed, 2 = Failed, 3 = Finalized
            Ok(0) // Pending (default for unknown messages)
        }));
    }

    /// Call a host function by name
    pub fn call(&self, name: &str, ctx: &mut X3Context, args: &[u64]) -> Result<u64> {
        let func = self.functions.get(name)
            .ok_or_else(|| anyhow!("Unknown host function: {}", name))?;
        func(ctx, args)
    }

    /// Check if a function is registered
    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    /// Get list of registered function names
    pub fn function_names(&self) -> Vec<&str> {
        self.functions.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for HostFunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// EVM-specific host functions for token.x3 extern declarations
pub struct EvmHostFunctions;

impl EvmHostFunctions {
    pub fn register(registry: &mut HostFunctionRegistry) {
        const EVM_TOKEN_GAS: u64 = 3000;

        let token_balances1 = Arc::clone(&registry.token_balances);

        registry.functions.insert("evm_balance_of".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(EVM_TOKEN_GAS)?;
            
            if args.len() < 2 {
                return Err(anyhow!("evm_balance_of requires 2 args: addr, owner"));
            }
            
            let token_addr = args[0];
            let owner = args[1];
            
            // Construct token key: ([u8;32] token_id, [u8;32] owner)
            let mut token_key = [0u8; 32];
            token_key[0] = 0x00; // EVM namespace
            token_key[1..9].copy_from_slice(&token_addr.to_le_bytes());
            let mut owner_key = [0u8; 32];
            owner_key[0..8].copy_from_slice(&owner.to_le_bytes());
            
            // Query from token_balances storage
            if let Ok(tb) = token_balances1.read() {
                if let Some(balance) = tb.get(&(token_key, owner_key)) {
                    return Ok(*balance as u64);
                }
            }
            // Return 0 for unknown balances
            Ok(0)
        }));

        registry.functions.insert("evm_transfer".into(), Box::new(|ctx, args| {
            ctx.consume_gas(EVM_TOKEN_GAS * 2)?;
            
            if args.len() < 3 {
                return Err(anyhow!("evm_transfer requires 3 args: addr, to, amount"));
            }
            
            let amount = args[2] as u128;
            
            // Record state change
            let mut key = [0u8; 32];
            key[0..8].copy_from_slice(&args[0].to_le_bytes());
            key[8..16].copy_from_slice(&args[1].to_le_bytes());
            
            ctx.record_state_change(
                key,
                None,
                amount.to_le_bytes().to_vec(),
            );
            
            ctx.emit_log(
                "Transfer".into(),
                format!("to={}, amount={}", args[1], amount).into_bytes(),
            );
            
            Ok(1) // Success
        }));

        registry.functions.insert("evm_approve".into(), Box::new(|ctx, args| {
            ctx.consume_gas(EVM_TOKEN_GAS)?;
            
            if args.len() < 3 {
                return Err(anyhow!("evm_approve requires 3 args: addr, spender, amount"));
            }
            
            ctx.emit_log(
                "Approval".into(),
                format!("spender={}, amount={}", args[1], args[2]).into_bytes(),
            );
            
            Ok(1) // Success
        }));

        let token_balances2 = Arc::clone(&registry.token_balances);
        registry.functions.insert("evm_allowance".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(EVM_TOKEN_GAS)?;
            
            if args.len() < 3 {
                return Err(anyhow!("evm_allowance requires 3 args: addr, owner, spender"));
            }
            
            let token_addr = args[0];
            let owner = args[1];
            let spender = args[2];
            
            // Construct token key (first element)
            let mut token_key = [0u8; 32];
            token_key[0] = 0x01; // EVM allowance namespace
            token_key[1..9].copy_from_slice(&token_addr.to_le_bytes());
            // Hash owner+spender to create secondary key
            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(&owner.to_le_bytes());
            hasher.update(&spender.to_le_bytes());
            let hash = hasher.finalize();
            let mut owner_sp_key = [0u8; 32];
            owner_sp_key[0..23].copy_from_slice(&hash[0..23]);
            
            // Query from token_balances (used for allowances too)
            if let Ok(tb) = token_balances2.read() {
                if let Some(allowance) = tb.get(&(token_key, owner_sp_key)) {
                    return Ok(*allowance as u64);
                }
            }
            Ok(0) // No allowance set
        }));

        registry.functions.insert("evm_transfer_from".into(), Box::new(|ctx, args| {
            ctx.consume_gas(EVM_TOKEN_GAS * 2)?;
            
            if args.len() < 4 {
                return Err(anyhow!("evm_transfer_from requires 4 args: addr, from, to, amount"));
            }
            
            ctx.emit_log(
                "TransferFrom".into(),
                format!("from={}, to={}, amount={}", args[1], args[2], args[3]).into_bytes(),
            );
            
            Ok(1) // Success
        }));
    }
}

/// SVM-specific host functions for token.x3 extern declarations
pub struct SvmHostFunctions;

impl SvmHostFunctions {
    pub fn register(registry: &mut HostFunctionRegistry) {
        const SVM_TOKEN_GAS: u64 = 2500;

        let token_balances_svm = Arc::clone(&registry.token_balances);
        registry.functions.insert("svm_balance_of".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(SVM_TOKEN_GAS)?;
            
            if args.len() < 2 {
                return Err(anyhow!("svm_balance_of requires 2 args: mint, owner"));
            }
            
            let mint = args[0];
            let owner = args[1];
            
            // Construct token key tuple for SVM: (token, owner)
            let mut token_key = [0u8; 32];
            token_key[0] = 0x80; // SVM namespace (128+)
            token_key[1..9].copy_from_slice(&mint.to_le_bytes());
            let mut owner_key = [0u8; 32];
            owner_key[0..8].copy_from_slice(&owner.to_le_bytes());
            
            // Query from token_balances storage
            if let Ok(tb) = token_balances_svm.read() {
                if let Some(balance) = tb.get(&(token_key, owner_key)) {
                    return Ok(*balance as u64);
                }
            }
            Ok(0)
        }));

        registry.functions.insert("svm_transfer".into(), Box::new(|ctx, args| {
            ctx.consume_gas(SVM_TOKEN_GAS * 2)?;
            
            if args.len() < 3 {
                return Err(anyhow!("svm_transfer requires 3 args: mint, to, amount"));
            }
            
            let amount = args[2] as u128;
            
            // Record state change with SVM prefix
            let mut key = [0u8; 32];
            key[0] = 128; // SVM prefix (>= 128)
            key[1..9].copy_from_slice(&args[0].to_le_bytes());
            key[9..17].copy_from_slice(&args[1].to_le_bytes());
            
            ctx.record_state_change(
                key,
                None,
                amount.to_le_bytes().to_vec(),
            );
            
            ctx.emit_log(
                "SPLTransfer".into(),
                format!("to={}, amount={}", args[1], amount).into_bytes(),
            );
            
            Ok(1) // Success
        }));

        registry.functions.insert("svm_approve".into(), Box::new(|ctx, args| {
            ctx.consume_gas(SVM_TOKEN_GAS)?;
            
            if args.len() < 3 {
                return Err(anyhow!("svm_approve requires 3 args: mint, delegate, amount"));
            }
            
            ctx.emit_log(
                "SPLApproval".into(),
                format!("delegate={}, amount={}", args[1], args[2]).into_bytes(),
            );
            
            Ok(1) // Success
        }));

        let token_balances_svm2 = Arc::clone(&registry.token_balances);
        registry.functions.insert("svm_allowance".into(), Box::new(move |ctx, args| {
            ctx.consume_gas(SVM_TOKEN_GAS)?;
            
            if args.len() < 3 {
                return Err(anyhow!("svm_allowance requires 3 args: mint, owner, delegate"));
            }
            
            let mint = args[0];
            let owner = args[1];
            let delegate = args[2];
            
            // Construct token key tuple for SVM delegation: (token_key, owner_delegate_hash)
            let mut token_key = [0u8; 32];
            token_key[0] = 0x81; // SVM delegation namespace
            token_key[1..9].copy_from_slice(&mint.to_le_bytes());
            // Hash owner+delegate to fit in remaining bytes
            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(&owner.to_le_bytes());
            hasher.update(&delegate.to_le_bytes());
            let hash = hasher.finalize();
            let mut owner_delegate = [0u8; 32];
            owner_delegate[0..23].copy_from_slice(&hash[0..23]);
            
            // Query from token_balances (used for delegations too)
            if let Ok(tb) = token_balances_svm2.read() {
                if let Some(delegation) = tb.get(&(token_key, owner_delegate)) {
                    return Ok(*delegation as u64);
                }
            }
            Ok(0) // No delegation set
        }));

        registry.functions.insert("svm_transfer_from".into(), Box::new(|ctx, args| {
            ctx.consume_gas(SVM_TOKEN_GAS * 2)?;
            
            if args.len() < 4 {
                return Err(anyhow!("svm_transfer_from requires 4 args: mint, from, to, amount"));
            }
            
            ctx.emit_log(
                "SPLTransferFrom".into(),
                format!("from={}, to={}, amount={}", args[1], args[2], args[3]).into_bytes(),
            );
            
            Ok(1) // Success
        }));
    }
}

/// DEX host functions for dex.x3 extern declarations
pub struct DexHostFunctions;

impl DexHostFunctions {
    pub fn register(registry: &mut HostFunctionRegistry) {
        const DEX_GAS: u64 = 10000;

        registry.functions.insert("host_quote_swap".into(), Box::new(|ctx, args| {
            ctx.consume_gas(DEX_GAS)?;
            
            if args.len() < 2 {
                return Err(anyhow!("host_quote_swap requires 2 args: path_hash, amt_in"));
            }
            
            let amt_in = args[1] as u128;
            
            // Mock quote: 0.3% fee, 1:1 rate
            let fee = amt_in * 3 / 1000;
            let amt_out = amt_in - fee;
            
            Ok(amt_out as u64)
        }));

        registry.functions.insert("host_swap_exact_in".into(), Box::new(|ctx, args| {
            ctx.consume_gas(DEX_GAS * 5)?;
            
            if args.len() < 4 {
                return Err(anyhow!("host_swap_exact_in requires 4 args: path_hash, amt_in, min_out, to"));
            }
            
            let amt_in = args[1] as u128;
            let min_out = args[2] as u128;
            
            // Mock swap: 0.3% fee
            let fee = amt_in * 3 / 1000;
            let amt_out = amt_in - fee;
            
            if amt_out < min_out {
                return Err(anyhow!("Slippage exceeded: {} < {}", amt_out, min_out));
            }
            
            ctx.emit_log(
                "Swap".into(),
                format!("amt_in={}, amt_out={}, to={}", amt_in, amt_out, args[3]).into_bytes(),
            );
            
            Ok(amt_out as u64)
        }));

        registry.functions.insert("host_find_routes".into(), Box::new(|ctx, args| {
            ctx.consume_gas(DEX_GAS * 2)?;
            
            if args.len() < 3 {
                return Err(anyhow!("host_find_routes requires 3 args: in_sym, out_sym, amt_in"));
            }
            
            let in_sym = args[0];
            let out_sym = args[1];
            let amt_in = args[2] as u128;
            
            // Encode route information:
            // If in_sym == out_sym: no route needed (same token)
            // Otherwise, encode a direct route: in_sym -> out_sym
            // Return format: num_routes (8 bits) | route_type (8 bits) | estimated_out (48 bits)
            
            if in_sym == out_sym {
                // Same token, no swap needed
                return Ok(0); // No routes
            }
            
            // Calculate estimated output with 0.3% fee
            let fee = amt_in * 3 / 1000;
            let estimated_out = amt_in - fee;
            
            // Encode single direct route
            // Format: routes=1, type=1 (direct), estimated_out in remaining bits
            let encoded: u64 = 1 // num_routes
                | (1 << 8) // route_type: 1 = direct
                | ((estimated_out as u64 & 0xFFFFFFFFFFFF) << 16); // 48-bit output estimate
            
            ctx.emit_log(
                "RouteFound".into(),
                format!("in={}, out={}, amt={}, est_out={}", in_sym, out_sym, amt_in, estimated_out).into_bytes(),
            );
            
            Ok(encoded)
        }));
    }
}

/// Create a fully-initialized host function registry with all modules
pub fn create_full_registry() -> HostFunctionRegistry {
    let mut registry = HostFunctionRegistry::new();
    EvmHostFunctions::register(&mut registry);
    SvmHostFunctions::register(&mut registry);
    DexHostFunctions::register(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = HostFunctionRegistry::new();
        
        assert!(registry.has_function("host_get_chain_id"));
        assert!(registry.has_function("host_get_block_height"));
        assert!(registry.has_function("caller_address"));
        assert!(registry.has_function("self_address"));
        assert!(registry.has_function("host_send_message"));
        assert!(registry.has_function("host_verify_proof"));
        assert!(registry.has_function("host_sha256"));
    }

    #[test]
    fn test_host_sha256_memory_roundtrip() {
        // Prepare WASM memory with input data
        let mut mem = InMemoryWasm::new(1024);
        let input = b"hello-host-sha";
        let ptr: u32 = 100;
        let out_ptr: u32 = 200;
        mem.write(ptr, input).unwrap();

        // Create registry with memory
        let registry = HostFunctionRegistry::new().with_wasm_memory(Arc::new(mem));
        let caller = [2u8; 32];
        let mut ctx = X3Context::new(caller, 100000);

        let res = registry.call("host_sha256", &mut ctx, &[ptr as u64, input.len() as u64, out_ptr as u64]).unwrap();
        assert_eq!(res, 1);

        // Read back digest from memory
        let digest = registry.read_wasm_memory(out_ptr, 32).unwrap();
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(input);
        let expected = hasher.finalize();
        assert_eq!(digest, expected.to_vec());
    }

    #[test]
    fn test_htlc_create_and_claim() {
        // Set up memory: secret hash + preimage
        let mut mem = InMemoryWasm::new(2048);
        let preimage = b"super-secret-pre".to_vec();
        let pre_ptr: u32 = 256;
        let pre_len: u32 = preimage.len() as u32;
        mem.write(pre_ptr, &preimage).unwrap();

        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&preimage);
        let secret = hasher.finalize();
        let secret_ptr: u32 = 512;
        mem.write(secret_ptr, &secret).unwrap();

        // Registry with memory
        let registry = HostFunctionRegistry::new().with_wasm_memory(Arc::new(mem));
        let caller = [9u8; 32];
        let mut ctx = X3Context::new(caller, 100000);

        // Create HTLC id=1
        let res = registry.call("host_htlc_create", &mut ctx, &[1u64, secret_ptr as u64, 32u64, 1000u64, 999u64]).unwrap();
        assert_eq!(res, 1);

        // Try claiming with correct preimage
        let res2 = registry.call("host_htlc_claim", &mut ctx, &[1u64, pre_ptr as u64, pre_len as u64]).unwrap();
        assert_eq!(res2, 1);

        // Check store shows claimed
        let store = registry.htlc_store.read().unwrap();
        let e = store.get(&1).expect("htlc exists");
        assert!(e.claimed);
        assert_eq!(e.claimed_by.unwrap(), caller);
    }

    #[test]
    fn test_core_functions() {
        let registry = HostFunctionRegistry::new();
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 10000);
        ctx.chain_id = 42;
        ctx.block_height = 100;
        
        // Test chain_id
        let chain_id = registry.call("host_get_chain_id", &mut ctx, &[]).unwrap();
        assert_eq!(chain_id, 42);
        
        // Test block_height
        let height = registry.call("host_get_block_height", &mut ctx, &[]).unwrap();
        assert_eq!(height, 100);
    }

    #[test]
    fn test_bridge_message() {
        let registry = HostFunctionRegistry::new();
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 100000);
        ctx.chain_id = 1;
        
        // Send a bridge message
        let result = registry.call(
            "host_send_message",
            &mut ctx,
            &[2, 0, 100000], // dst_chain=2, payload_ptr=0, gas_limit=100000
        ).unwrap();
        
        assert_ne!(result, 0); // Should return non-zero msg_id
        assert_eq!(ctx.bridge_messages.len(), 1);
        assert_eq!(ctx.bridge_messages[0].dst_chain, 2);
    }

    #[test]
    fn test_full_registry() {
        let registry = create_full_registry();
        
        // Core functions
        assert!(registry.has_function("host_get_chain_id"));
        
        // Bridge functions
        assert!(registry.has_function("host_send_message"));
        
        // EVM token functions
        assert!(registry.has_function("evm_balance_of"));
        assert!(registry.has_function("evm_transfer"));
        
        // SVM token functions
        assert!(registry.has_function("svm_balance_of"));
        assert!(registry.has_function("svm_transfer"));
        
        // DEX functions
        assert!(registry.has_function("host_quote_swap"));
        assert!(registry.has_function("host_swap_exact_in"));
    }

    #[test]
    fn test_evm_transfer() {
        let mut registry = HostFunctionRegistry::new();
        EvmHostFunctions::register(&mut registry);
        
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 100000);
        
        let result = registry.call(
            "evm_transfer",
            &mut ctx,
            &[100, 200, 1000], // addr, to, amount
        ).unwrap();
        
        assert_eq!(result, 1); // Success
        assert_eq!(ctx.logs.len(), 1);
        assert_eq!(ctx.logs[0].topic, "Transfer");
        assert_eq!(ctx.state_changes.len(), 1);
    }

    #[test]
    fn test_dex_quote() {
        let mut registry = HostFunctionRegistry::new();
        DexHostFunctions::register(&mut registry);
        
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 100000);
        
        let result = registry.call(
            "host_quote_swap",
            &mut ctx,
            &[0, 1000000], // path_hash, amt_in
        ).unwrap();
        
        // 0.3% fee: 1000000 - 3000 = 997000
        assert_eq!(result, 997000);
    }

    #[test]
    fn test_dex_swap_slippage() {
        let mut registry = HostFunctionRegistry::new();
        DexHostFunctions::register(&mut registry);
        
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 100000);
        
        // Try swap with min_out too high (should fail)
        let result = registry.call(
            "host_swap_exact_in",
            &mut ctx,
            &[0, 1000, 2000, 300], // path_hash, amt_in=1000, min_out=2000, to
        );
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Slippage"));
    }
}
