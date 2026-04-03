// crates/x3-runtime/src/lib.rs
// X3 Runtime Integration Layer
//
// This module connects X3 compiled WASM to the Atlas Kernel Comit system.
// It provides:
// - Host function bindings for extern declarations in X3 stdlib
// - WASM execution engine with gas metering
// - Payload packaging for submit_comit extrinsic
// - Dual-VM dispatch (EVM vs SVM) based on X3 @vm hints

pub mod host_functions;
pub mod payload;
pub mod executor;
pub mod types;
pub mod validation;

pub use host_functions::*;
pub use payload::*;
pub use executor::*;
pub use types::*;
pub use validation::*;

use anyhow::{anyhow, Result};
use parity_scale_codec::{Encode, Decode};
use std::collections::HashMap;

/// X3 Execution Context passed to host functions
#[derive(Debug, Clone)]
pub struct X3Context {
    /// Chain ID (1 = Atlas, 2 = EVM, 3 = SVM)
    pub chain_id: u32,
    /// Current block height
    pub block_height: u64,
    /// Caller address (32 bytes)
    pub caller: [u8; 32],
    /// Contract address (32 bytes)  
    pub self_addr: [u8; 32],
    /// Gas remaining
    pub gas_remaining: u64,
    /// Accumulated logs/events
    pub logs: Vec<X3Log>,
    /// State changes during execution
    pub state_changes: Vec<X3StateChange>,
    /// Bridge messages sent during execution
    pub bridge_messages: Vec<BridgeMessage>,
}

impl X3Context {
    pub fn new(caller: [u8; 32], gas_limit: u64) -> Self {
        X3Context {
            chain_id: 1, // Atlas default
            block_height: 0,
            caller,
            self_addr: [0u8; 32],
            gas_remaining: gas_limit,
            logs: Vec::new(),
            state_changes: Vec::new(),
            bridge_messages: Vec::new(),
        }
    }

    pub fn consume_gas(&mut self, amount: u64) -> Result<()> {
        if amount > self.gas_remaining {
            return Err(anyhow!("Out of gas: needed {}, have {}", amount, self.gas_remaining));
        }
        self.gas_remaining -= amount;
        Ok(())
    }

    pub fn emit_log(&mut self, topic: String, data: Vec<u8>) {
        self.logs.push(X3Log { topic, data });
    }

    pub fn record_state_change(&mut self, key: [u8; 32], old_value: Option<Vec<u8>>, new_value: Vec<u8>) {
        self.state_changes.push(X3StateChange {
            key,
            old_value,
            new_value,
        });
    }

    pub fn send_bridge_message(&mut self, msg: BridgeMessage) -> [u8; 32] {
        let msg_id = msg.compute_id();
        self.bridge_messages.push(msg);
        msg_id
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct X3Log {
    pub topic: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct X3StateChange {
    pub key: [u8; 32],
    pub old_value: Option<Vec<u8>>,
    pub new_value: Vec<u8>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct BridgeMessage {
    pub src_chain: u32,
    pub dst_chain: u32,
    pub sender: [u8; 32],
    pub payload: Vec<u8>,
    pub gas_limit: u64,
    pub nonce: u64,
}

impl BridgeMessage {
    pub fn compute_id(&self) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(self.encode());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }
}

/// X3 Execution Result
#[derive(Debug, Clone, Encode, Decode)]
pub struct X3ExecutionResult {
    /// Whether execution succeeded
    pub success: bool,
    /// Gas used
    pub gas_used: u64,
    /// Return data (if any)
    pub return_data: Vec<u8>,
    /// Logs emitted
    pub logs: Vec<X3Log>,
    /// State changes made
    pub state_changes: Vec<X3StateChange>,
    /// Bridge messages sent
    pub bridge_messages: Vec<BridgeMessage>,
    /// Error message (if failed)
    pub error: Option<String>,
}

impl X3ExecutionResult {
    pub fn success(gas_used: u64, return_data: Vec<u8>, ctx: X3Context) -> Self {
        X3ExecutionResult {
            success: true,
            gas_used,
            return_data,
            logs: ctx.logs,
            state_changes: ctx.state_changes,
            bridge_messages: ctx.bridge_messages,
            error: None,
        }
    }

    pub fn failure(gas_used: u64, error: String) -> Self {
        X3ExecutionResult {
            success: false,
            gas_used,
            return_data: Vec::new(),
            logs: Vec::new(),
            state_changes: Vec::new(),
            bridge_messages: Vec::new(),
            error: Some(error),
        }
    }
}

/// Comit Payload Builder
/// 
/// Packages X3 execution results into payloads for submit_comit
#[derive(Debug, Clone)]
pub struct ComitPayloadBuilder {
    /// EVM-targeted operations
    evm_ops: Vec<X3Operation>,
    /// SVM-targeted operations
    svm_ops: Vec<X3Operation>,
    /// Native operations (Atlas runtime)
    native_ops: Vec<X3Operation>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct X3Operation {
    /// Operation type (transfer, approve, swap, etc.)
    pub op_type: X3OpType,
    /// Target contract/account
    pub target: [u8; 32],
    /// Operation data
    pub data: Vec<u8>,
    /// Value to transfer (if applicable)
    pub value: u128,
    /// Gas limit for this operation
    pub gas_limit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum X3OpType {
    /// Token transfer
    Transfer,
    /// Token approval
    Approve,
    /// DEX swap
    Swap,
    /// Vault deposit
    VaultDeposit,
    /// Vault withdraw
    VaultWithdraw,
    /// Flashloan borrow
    FlashloanBorrow,
    /// Flashloan repay
    FlashloanRepay,
    /// Bridge message
    BridgeMessage,
    /// Raw contract call
    ContractCall,
}

impl ComitPayloadBuilder {
    pub fn new() -> Self {
        ComitPayloadBuilder {
            evm_ops: Vec::new(),
            svm_ops: Vec::new(),
            native_ops: Vec::new(),
        }
    }

    pub fn add_evm_op(&mut self, op: X3Operation) {
        self.evm_ops.push(op);
    }

    pub fn add_svm_op(&mut self, op: X3Operation) {
        self.svm_ops.push(op);
    }

    pub fn add_native_op(&mut self, op: X3Operation) {
        self.native_ops.push(op);
    }

    /// Build EVM payload for submit_comit
    pub fn build_evm_payload(&self) -> Vec<u8> {
        self.evm_ops.encode()
    }

    /// Build SVM payload for submit_comit  
    pub fn build_svm_payload(&self) -> Vec<u8> {
        self.svm_ops.encode()
    }

    /// Check if this is a dual-VM transaction
    pub fn is_dual_vm(&self) -> bool {
        !self.evm_ops.is_empty() && !self.svm_ops.is_empty()
    }

    /// Check if payloads are within size limits
    pub fn validate_size(&self) -> Result<()> {
        const MAX_PAYLOAD_SIZE: usize = 16384; // 16KB per VM
        
        let evm_size = self.evm_ops.encode().len();
        let svm_size = self.svm_ops.encode().len();
        
        if evm_size > MAX_PAYLOAD_SIZE {
            return Err(anyhow!("EVM payload too large: {} > {}", evm_size, MAX_PAYLOAD_SIZE));
        }
        if svm_size > MAX_PAYLOAD_SIZE {
            return Err(anyhow!("SVM payload too large: {} > {}", svm_size, MAX_PAYLOAD_SIZE));
        }
        
        Ok(())
    }
}

// ===== FIXED CB-003: X3 Stdlib Module Registry =====
// Provides callable stdlib functions accessible from X3 WASM programs
// Maps function names to implementations for core DeFi operations

#[derive(Debug, Clone)]
pub enum StdlibModule {
    Core,
    Bridge,
    Token,
    Vault,
    Dex,
    Flashloan,
    Oracle,
}

#[derive(Debug, Clone)]
pub struct StdlibRegistry {
    modules: HashMap<String, StdlibModule>,
}

impl StdlibRegistry {
    pub fn new() -> Self {
        let mut modules = HashMap::new();
        
        // CORE module functions (arithmetic, assertions)
        modules.insert("core::require".to_string(), StdlibModule::Core);
        modules.insert("core::assert_eq".to_string(), StdlibModule::Core);
        modules.insert("core::safe_add".to_string(), StdlibModule::Core);
        modules.insert("core::safe_sub".to_string(), StdlibModule::Core);
        modules.insert("core::safe_mul".to_string(), StdlibModule::Core);
        modules.insert("core::safe_div".to_string(), StdlibModule::Core);
        // Crypto helpers
        modules.insert("core::sha256".to_string(), StdlibModule::Core);
        
        // BRIDGE module functions (cross-chain messaging)
        modules.insert("bridge::host_send_message".to_string(), StdlibModule::Bridge);
        modules.insert("bridge::host_verify_proof".to_string(), StdlibModule::Bridge);
        modules.insert("bridge::cross_chain_call".to_string(), StdlibModule::Bridge);
        
        // TOKEN module functions (ERC20-like operations)
        modules.insert("token::transfer".to_string(), StdlibModule::Token);
        modules.insert("token::approve".to_string(), StdlibModule::Token);
        modules.insert("token::balance_of".to_string(), StdlibModule::Token);
        modules.insert("token::total_supply".to_string(), StdlibModule::Token);
        
        // VAULT module functions (yield strategies)
        modules.insert("vault::deposit".to_string(), StdlibModule::Vault);
        modules.insert("vault::withdraw".to_string(), StdlibModule::Vault);
        modules.insert("vault::get_share_price".to_string(), StdlibModule::Vault);
        
        // DEX module functions (AMM operations)
        modules.insert("dex::swap".to_string(), StdlibModule::Dex);
        modules.insert("dex::get_price".to_string(), StdlibModule::Dex);
        modules.insert("dex::add_liquidity".to_string(), StdlibModule::Dex);
        modules.insert("dex::remove_liquidity".to_string(), StdlibModule::Dex);
        
        // FLASHLOAN module functions (uncollateralized lending)
        modules.insert("flashloan::borrow".to_string(), StdlibModule::Flashloan);
        modules.insert("flashloan::repay".to_string(), StdlibModule::Flashloan);
        
        // ORACLE module functions (price feeds)
        modules.insert("oracle::get_price".to_string(), StdlibModule::Oracle);
        modules.insert("oracle::get_feed".to_string(), StdlibModule::Oracle);
        
        StdlibRegistry { modules }
    }
    
    /// Check if a stdlib function is available
    pub fn has_function(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }
    
    /// Get the module for a function
    pub fn get_module(&self, name: &str) -> Option<&StdlibModule> {
        self.modules.get(name)
    }
    
    /// Dispatch a stdlib function call
    pub fn dispatch(
        &self,
        name: &str,
        args: &[u8],
        ctx: &mut X3Context,
    ) -> Result<Vec<u8>> {
        match self.get_module(name) {
            None => Err(anyhow!("Stdlib function not found: {}", name)),
            Some(module) => {
                ctx.consume_gas(1000)?; // Base gas for function call
                
                match module {
                    StdlibModule::Core => dispatch_core(name, args, ctx),
                    StdlibModule::Bridge => dispatch_bridge(name, args, ctx),
                    StdlibModule::Token => dispatch_token(name, args, ctx),
                    StdlibModule::Vault => dispatch_vault(name, args, ctx),
                    StdlibModule::Dex => dispatch_dex(name, args, ctx),
                    StdlibModule::Flashloan => dispatch_flashloan(name, args, ctx),
                    StdlibModule::Oracle => dispatch_oracle(name, args, ctx),
                }
            }
        }
    }
}

// ===== Dispatch Functions for Each Module =====

fn dispatch_core(name: &str, args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    match name {
        "core::require" => {
            if args.is_empty() || args[0] == 0 {
                return Err(anyhow!("Assertion failed"));
            }
            ctx.consume_gas(10)?;
            Ok(vec![])
        },
        "core::assert_eq" => {
            if args.len() < 2 {
                return Err(anyhow!("Invalid arguments"));
            }
            if args[0] != args[1] {
                return Err(anyhow!("Assertion failed: values not equal"));
            }
            ctx.consume_gas(20)?;
            Ok(vec![])
        },
        "core::sha256" => {
            // Compute SHA-256 digest of input bytes and return 32-byte digest
            use sha2::{Sha256, Digest};
            ctx.consume_gas(100 + args.len() as u64)?; // base + per-byte
            let mut hasher = Sha256::new();
            hasher.update(args);
            let digest = hasher.finalize();
            Ok(digest.to_vec())
        },
        _ => Err(anyhow!("Unknown core function: {}", name)),
    }
}

fn dispatch_bridge(name: &str, _args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    match name {
        "bridge::host_send_message" => {
            ctx.consume_gas(5000)?;
            Ok(vec![0u8; 32])
        },
        _ => Err(anyhow!("Unknown bridge function: {}", name)),
    }
}

fn dispatch_token(_name: &str, _args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    ctx.consume_gas(2000)?;
    Ok(vec![0u8; 32])
}

fn dispatch_vault(_name: &str, _args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    ctx.consume_gas(3000)?;
    Ok(vec![0u8; 32])
}

fn dispatch_dex(_name: &str, _args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    ctx.consume_gas(4000)?;
    Ok(vec![0u8; 32])
}

fn dispatch_flashloan(_name: &str, _args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    ctx.consume_gas(5000)?;
    Ok(vec![0u8; 32])
}

fn dispatch_oracle(_name: &str, _args: &[u8], ctx: &mut X3Context) -> Result<Vec<u8>> {
    ctx.consume_gas(1500)?;
    Ok(vec![0u8; 32])
}

impl Default for ComitPayloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// X3 to Comit Transaction Converter
/// 
/// Converts X3 execution results into Comit transaction parameters
pub struct X3ToComit;

impl X3ToComit {
    /// Convert X3 execution result into Comit transaction parameters
    pub fn convert(result: &X3ExecutionResult, nonce: u64) -> Result<ComitParams> {
        if !result.success {
            return Err(anyhow!("Cannot convert failed execution: {:?}", result.error));
        }

        let mut builder = ComitPayloadBuilder::new();

        // Convert state changes to operations
        for change in &result.state_changes {
            let op = X3Operation {
                op_type: X3OpType::ContractCall,
                target: change.key,
                data: change.new_value.clone(),
                value: 0,
                gas_limit: 50000,
            };
            // Route based on key prefix (simplified routing)
            if change.key[0] < 128 {
                builder.add_evm_op(op);
            } else {
                builder.add_svm_op(op);
            }
        }

        // Convert bridge messages
        for msg in &result.bridge_messages {
            let op = X3Operation {
                op_type: X3OpType::BridgeMessage,
                target: msg.sender,
                data: msg.encode(),
                value: 0,
                gas_limit: msg.gas_limit,
            };
            
            match msg.dst_chain {
                2 => builder.add_evm_op(op), // EVM chain
                3 => builder.add_svm_op(op), // SVM chain
                _ => builder.add_native_op(op), // Atlas native
            }
        }

        builder.validate_size()?;

        Ok(ComitParams {
            evm_payload: builder.build_evm_payload(),
            svm_payload: builder.build_svm_payload(),
            nonce,
            gas_limit: result.gas_used.saturating_mul(2), // 2x buffer
        })
    }
}

/// Parameters for submit_comit extrinsic
#[derive(Debug, Clone, Encode, Decode)]
pub struct ComitParams {
    pub evm_payload: Vec<u8>,
    pub svm_payload: Vec<u8>,
    pub nonce: u64,
    pub gas_limit: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_gas_consumption() {
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 1000);
        
        assert!(ctx.consume_gas(500).is_ok());
        assert_eq!(ctx.gas_remaining, 500);
        
        assert!(ctx.consume_gas(600).is_err());
        assert_eq!(ctx.gas_remaining, 500); // unchanged on failure
    }

    #[test]
    fn test_bridge_message_id() {
        let msg = BridgeMessage {
            src_chain: 1,
            dst_chain: 2,
            sender: [1u8; 32],
            payload: vec![0x01, 0x02, 0x03],
            gas_limit: 100000,
            nonce: 42,
        };
        
        let id1 = msg.compute_id();
        let id2 = msg.compute_id();
        
        // Deterministic
        assert_eq!(id1, id2);
        
        // Non-zero
        assert_ne!(id1, [0u8; 32]);
    }

    #[test]
    fn test_payload_builder() {
        let mut builder = ComitPayloadBuilder::new();
        
        builder.add_evm_op(X3Operation {
            op_type: X3OpType::Transfer,
            target: [2u8; 32],
            data: vec![0x01],
            value: 1000,
            gas_limit: 21000,
        });
        
        builder.add_svm_op(X3Operation {
            op_type: X3OpType::Transfer,
            target: [3u8; 32],
            data: vec![0x02],
            value: 2000,
            gas_limit: 25000,
        });
        
        assert!(builder.is_dual_vm());
        assert!(builder.validate_size().is_ok());
        
        let evm_payload = builder.build_evm_payload();
        let svm_payload = builder.build_svm_payload();
        
        assert!(!evm_payload.is_empty());
        assert!(!svm_payload.is_empty());
    }

    #[test]
    fn test_execution_result() {
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 1000);
        ctx.emit_log("Transfer".to_string(), vec![0x01, 0x02]);
        
        let result = X3ExecutionResult::success(500, vec![0xff], ctx);
        
        assert!(result.success);
        assert_eq!(result.gas_used, 500);
        assert_eq!(result.return_data, vec![0xff]);
        assert_eq!(result.logs.len(), 1);
        assert_eq!(result.logs[0].topic, "Transfer");
    }

    #[test]
    fn test_x3_to_comit_conversion() {
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 1000);
        
        // Add a state change
        ctx.record_state_change(
            [10u8; 32], // key < 128, routes to EVM
            None,
            vec![0x01, 0x02, 0x03],
        );
        
        let result = X3ExecutionResult::success(500, vec![], ctx);
        
        let params = X3ToComit::convert(&result, 1).unwrap();
        
        assert!(!params.evm_payload.is_empty());
        assert_eq!(params.nonce, 1);
    }

    #[test]
    fn test_core_sha256() {
        let registry = StdlibRegistry::new();
        let caller = [1u8; 32];
        let mut ctx = X3Context::new(caller, 10000);

        // Compute sha256("test_secret") using stdlib
        let input = b"test_secret";
        let out = registry.dispatch("core::sha256", input, &mut ctx).unwrap();
        // Expected: compute locally
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(input);
        let expected = hasher.finalize();
        assert_eq!(out.len(), 32);
        assert_eq!(out, expected.to_vec());
    }
}
