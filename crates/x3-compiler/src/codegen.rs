// crates/x3-compiler/src/codegen.rs
// Code generation: X3 AST -> WASM + host ABI
// Produces valid WASM bytecode with:
// - Type section (function signatures)
// - Import section (host function bindings)
// - Function section (user functions)
// - Export section (public API)
// - Code section (instruction streams)
// - Data section (static constants)
// - Memory section (linear memory)
// - Global section (mutable globals)

use crate::analyzer::{AnalyzedProgram, AnalyzedModule, AnalyzedItem};
use crate::ast::{Function, Statement, Expr, Literal, BinOp, UnOp, Type, StructDef, Strategy};
use anyhow::{Result, anyhow};
use log::info;
use std::collections::HashMap;

/// WASM opcodes
mod opcodes {
    // Control flow
    pub const UNREACHABLE: u8 = 0x00;
    pub const NOP: u8 = 0x01;
    pub const BLOCK: u8 = 0x02;
    pub const LOOP: u8 = 0x03;
    pub const IF: u8 = 0x04;
    pub const ELSE: u8 = 0x05;
    pub const END: u8 = 0x0b;
    pub const BR: u8 = 0x0c;
    pub const BR_IF: u8 = 0x0d;
    pub const BR_TABLE: u8 = 0x0e;
    pub const RETURN: u8 = 0x0f;
    pub const CALL: u8 = 0x10;
    pub const CALL_INDIRECT: u8 = 0x11;
    
    // Parametric
    pub const DROP: u8 = 0x1a;
    pub const SELECT: u8 = 0x1b;
    
    // Variable access
    pub const LOCAL_GET: u8 = 0x20;
    pub const LOCAL_SET: u8 = 0x21;
    pub const LOCAL_TEE: u8 = 0x22;
    pub const GLOBAL_GET: u8 = 0x23;
    pub const GLOBAL_SET: u8 = 0x24;
    
    // Memory
    pub const I32_LOAD: u8 = 0x28;
    pub const I64_LOAD: u8 = 0x29;
    pub const I32_LOAD8_S: u8 = 0x2c;
    pub const I32_LOAD8_U: u8 = 0x2d;
    pub const I32_LOAD16_S: u8 = 0x2e;
    pub const I32_LOAD16_U: u8 = 0x2f;
    pub const I64_LOAD8_S: u8 = 0x30;
    pub const I64_LOAD8_U: u8 = 0x31;
    pub const I64_LOAD16_S: u8 = 0x32;
    pub const I64_LOAD16_U: u8 = 0x33;
    pub const I64_LOAD32_S: u8 = 0x34;
    pub const I64_LOAD32_U: u8 = 0x35;
    pub const I32_STORE: u8 = 0x36;
    pub const I64_STORE: u8 = 0x37;
    pub const I32_STORE8: u8 = 0x3a;
    pub const I32_STORE16: u8 = 0x3b;
    pub const I64_STORE8: u8 = 0x3c;
    pub const I64_STORE16: u8 = 0x3d;
    pub const I64_STORE32: u8 = 0x3e;
    pub const MEMORY_SIZE: u8 = 0x3f;
    pub const MEMORY_GROW: u8 = 0x40;
    
    // Constants
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const F32_CONST: u8 = 0x43;
    pub const F64_CONST: u8 = 0x44;
    
    // Comparison (i32)
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_NE: u8 = 0x47;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_LT_U: u8 = 0x49;
    pub const I32_GT_S: u8 = 0x4a;
    pub const I32_GT_U: u8 = 0x4b;
    pub const I32_LE_S: u8 = 0x4c;
    pub const I32_LE_U: u8 = 0x4d;
    pub const I32_GE_S: u8 = 0x4e;
    pub const I32_GE_U: u8 = 0x4f;
    
    // Comparison (i64)
    pub const I64_EQZ: u8 = 0x50;
    pub const I64_EQ: u8 = 0x51;
    pub const I64_NE: u8 = 0x52;
    pub const I64_LT_S: u8 = 0x53;
    pub const I64_LT_U: u8 = 0x54;
    pub const I64_GT_S: u8 = 0x55;
    pub const I64_GT_U: u8 = 0x56;
    pub const I64_LE_S: u8 = 0x57;
    pub const I64_LE_U: u8 = 0x58;
    pub const I64_GE_S: u8 = 0x59;
    pub const I64_GE_U: u8 = 0x5a;
    
    // Arithmetic (i32)
    pub const I32_CLZ: u8 = 0x67;
    pub const I32_CTZ: u8 = 0x68;
    pub const I32_POPCNT: u8 = 0x69;
    pub const I32_ADD: u8 = 0x6a;
    pub const I32_SUB: u8 = 0x6b;
    pub const I32_MUL: u8 = 0x6c;
    pub const I32_DIV_S: u8 = 0x6d;
    pub const I32_DIV_U: u8 = 0x6e;
    pub const I32_REM_S: u8 = 0x6f;
    pub const I32_REM_U: u8 = 0x70;
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    pub const I32_SHL: u8 = 0x74;
    pub const I32_SHR_S: u8 = 0x75;
    pub const I32_SHR_U: u8 = 0x76;
    pub const I32_ROTL: u8 = 0x77;
    pub const I32_ROTR: u8 = 0x78;
    
    // Arithmetic (i64)
    pub const I64_CLZ: u8 = 0x79;
    pub const I64_CTZ: u8 = 0x7a;
    pub const I64_POPCNT: u8 = 0x7b;
    pub const I64_ADD: u8 = 0x7c;
    pub const I64_SUB: u8 = 0x7d;
    pub const I64_MUL: u8 = 0x7e;
    pub const I64_DIV_S: u8 = 0x7f;
    pub const I64_DIV_U: u8 = 0x80;
    pub const I64_REM_S: u8 = 0x81;
    pub const I64_REM_U: u8 = 0x82;
    pub const I64_AND: u8 = 0x83;
    pub const I64_OR: u8 = 0x84;
    pub const I64_XOR: u8 = 0x85;
    pub const I64_SHL: u8 = 0x86;
    pub const I64_SHR_S: u8 = 0x87;
    pub const I64_SHR_U: u8 = 0x88;
    pub const I64_ROTL: u8 = 0x89;
    pub const I64_ROTR: u8 = 0x8a;
    
    // Conversions
    pub const I32_WRAP_I64: u8 = 0xa7;
    pub const I64_EXTEND_I32_S: u8 = 0xac;
    pub const I64_EXTEND_I32_U: u8 = 0xad;
    
    // Types
    pub const TYPE_I32: u8 = 0x7f;
    pub const TYPE_I64: u8 = 0x7e;
    pub const TYPE_F32: u8 = 0x7d;
    pub const TYPE_F64: u8 = 0x7c;
    pub const TYPE_VOID: u8 = 0x40;
    pub const TYPE_FUNC: u8 = 0x60;
}

/// WASM section IDs
mod sections {
    pub const CUSTOM: u8 = 0;
    pub const TYPE: u8 = 1;
    pub const IMPORT: u8 = 2;
    pub const FUNCTION: u8 = 3;
    pub const TABLE: u8 = 4;
    pub const MEMORY: u8 = 5;
    pub const GLOBAL: u8 = 6;
    pub const EXPORT: u8 = 7;
    pub const START: u8 = 8;
    pub const ELEMENT: u8 = 9;
    pub const CODE: u8 = 10;
    pub const DATA: u8 = 11;
}

/// Intermediate representation for WASM instructions
#[derive(Debug, Clone)]
pub enum WasmInstr {
    // Constants
    I64Const(i64),
    I32Const(i32),
    
    // Arithmetic (i64)
    I64Add,
    I64Sub,
    I64Mul,
    I64DivU,
    I64DivS,
    I64RemU,
    I64RemS,
    
    // Bitwise (i64)
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrU,
    I64ShrS,
    
    // Comparisons (i64)
    I64Eqz,
    I64Eq,
    I64Ne,
    I64LtU,
    I64LtS,
    I64GtU,
    I64GtS,
    I64LeU,
    I64LeS,
    I64GeU,
    I64GeS,
    
    // Arithmetic (i32)
    I32Add,
    I32Sub,
    I32Mul,
    I32DivU,
    I32DivS,
    I32RemU,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrU,
    I32ShrS,
    
    // Comparisons (i32)
    I32Eqz,
    I32Eq,
    I32Ne,
    I32LtU,
    I32LtS,
    I32GtU,
    I32GtS,
    I32LeU,
    I32LeS,
    I32GeU,
    I32GeS,
    
    // Control flow
    Unreachable,
    Block { result_type: Option<u8> },
    Loop { result_type: Option<u8> },
    If { result_type: Option<u8> },
    Else,
    End,
    Br { label: u32 },
    BrIf { label: u32 },
    BrTable { labels: Vec<u32>, default: u32 },
    Return,
    Call { func_idx: u32 },
    CallIndirect { type_idx: u32 },
    
    // Memory operations
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    I64Load { align: u32, offset: u32 },
    I64Store { align: u32, offset: u32 },
    I32Load { align: u32, offset: u32 },
    I32Store { align: u32, offset: u32 },
    I32Load8U { align: u32, offset: u32 },
    I32Store8 { align: u32, offset: u32 },
    MemorySize,
    MemoryGrow,
    
    // Type conversions
    I32WrapI64,
    I64ExtendI32U,
    I64ExtendI32S,
    
    // Stack operations
    Drop,
    Select,
    Nop,
}

/// Code generator state
pub struct Codegen {
    code_sections: Vec<Vec<u8>>,  // Function bodies
    locals_stack: Vec<Vec<(String, u8)>>,  // Local variables per function
    type_signatures: Vec<(Vec<u8>, Vec<u8>)>,  // (params, results)
    imports: Vec<(String, String, u32)>,  // (module, name, type_idx)
    exports: Vec<(String, u32, u8)>,  // (name, index, kind)
    memory_pages: u32,
    globals: Vec<(u8, bool, Vec<u8>)>,  // (type, mutable, init_expr)
}

impl Codegen {
    fn new() -> Self {
        // Standard WASM type signatures for bridge operations
        let type_signatures = vec![
            (vec![opcodes::TYPE_I64], vec![opcodes::TYPE_I64]),  // Type 0: (i64) -> i64
            (vec![opcodes::TYPE_I64, opcodes::TYPE_I64], vec![opcodes::TYPE_I64]),  // Type 1: (i64, i64) -> i64
            (vec![], vec![opcodes::TYPE_I64]),  // Type 2: () -> i64
            (vec![], vec![]),  // Type 3: () -> ()
            (vec![opcodes::TYPE_I32], vec![opcodes::TYPE_I32]),  // Type 4: (i32) -> i32
            (vec![opcodes::TYPE_I32, opcodes::TYPE_I32], vec![opcodes::TYPE_I32]),  // Type 5: (i32, i32) -> i32
            (vec![opcodes::TYPE_I64, opcodes::TYPE_I64, opcodes::TYPE_I64], vec![opcodes::TYPE_I64]),  // Type 6: (i64, i64, i64) -> i64
        ];

        // Host import functions (order matters for call indices)
        let imports = vec![
            ("env".to_string(), "host_send_message".to_string(), 1),
            ("env".to_string(), "host_verify_proof".to_string(), 1),
            ("env".to_string(), "host_execute_intent".to_string(), 0),
            ("env".to_string(), "host_finalize_intent".to_string(), 0),
            ("env".to_string(), "host_get_chain_id".to_string(), 2),
            ("env".to_string(), "host_get_block_height".to_string(), 2),
            ("env".to_string(), "caller_address".to_string(), 2),
            ("env".to_string(), "self_address".to_string(), 2),
            ("env".to_string(), "get_storage".to_string(), 1),
            ("env".to_string(), "set_storage".to_string(), 1),
            ("env".to_string(), "host_sha256".to_string(), 6),
        ];

        Codegen {
            code_sections: Vec::new(),
            locals_stack: vec![Vec::new()],
            type_signatures,
            imports,
            exports: Vec::new(),
            memory_pages: 1,
            globals: Vec::new(),
        }
    }

    /// Emit a single byte to code
    fn emit_byte(code: &mut Vec<u8>, byte: u8) {
        code.push(byte);
    }

    /// Emit unsigned LEB128 integer
    fn emit_leb128_unsigned(value: u64, code: &mut Vec<u8>) {
        let mut n = value;
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                code.push(byte | 0x80);
            } else {
                code.push(byte);
                break;
            }
        }
    }

    /// Emit signed LEB128 integer
    fn emit_leb128_signed(value: i64, code: &mut Vec<u8>) {
        let mut n = value;
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if (n == 0 && (byte & 0x40) == 0) || (n == -1 && (byte & 0x40) != 0) {
                code.push(byte);
                break;
            } else {
                code.push(byte | 0x80);
            }
        }
    }

    /// Emit a WASM instruction to code buffer
    fn emit_instr(code: &mut Vec<u8>, instr: &WasmInstr) {
        match instr {
            WasmInstr::I64Const(n) => {
                Self::emit_byte(code, opcodes::I64_CONST);
                Self::emit_leb128_signed(*n, code);
            }
            WasmInstr::I32Const(n) => {
                Self::emit_byte(code, opcodes::I32_CONST);
                Self::emit_leb128_signed(*n as i64, code);
            }

            // Arithmetic (i64)
            WasmInstr::I64Add => Self::emit_byte(code, opcodes::I64_ADD),
            WasmInstr::I64Sub => Self::emit_byte(code, opcodes::I64_SUB),
            WasmInstr::I64Mul => Self::emit_byte(code, opcodes::I64_MUL),
            WasmInstr::I64DivU => Self::emit_byte(code, opcodes::I64_DIV_U),
            WasmInstr::I64DivS => Self::emit_byte(code, opcodes::I64_DIV_S),
            WasmInstr::I64RemU => Self::emit_byte(code, opcodes::I64_REM_U),
            WasmInstr::I64RemS => Self::emit_byte(code, opcodes::I64_REM_S),

            // Bitwise (i64)
            WasmInstr::I64And => Self::emit_byte(code, opcodes::I64_AND),
            WasmInstr::I64Or => Self::emit_byte(code, opcodes::I64_OR),
            WasmInstr::I64Xor => Self::emit_byte(code, opcodes::I64_XOR),
            WasmInstr::I64Shl => Self::emit_byte(code, opcodes::I64_SHL),
            WasmInstr::I64ShrU => Self::emit_byte(code, opcodes::I64_SHR_U),
            WasmInstr::I64ShrS => Self::emit_byte(code, opcodes::I64_SHR_S),

            // Comparisons (i64)
            WasmInstr::I64Eqz => Self::emit_byte(code, opcodes::I64_EQZ),
            WasmInstr::I64Eq => Self::emit_byte(code, opcodes::I64_EQ),
            WasmInstr::I64Ne => Self::emit_byte(code, opcodes::I64_NE),
            WasmInstr::I64LtU => Self::emit_byte(code, opcodes::I64_LT_U),
            WasmInstr::I64LtS => Self::emit_byte(code, opcodes::I64_LT_S),
            WasmInstr::I64GtU => Self::emit_byte(code, opcodes::I64_GT_U),
            WasmInstr::I64GtS => Self::emit_byte(code, opcodes::I64_GT_S),
            WasmInstr::I64LeU => Self::emit_byte(code, opcodes::I64_LE_U),
            WasmInstr::I64LeS => Self::emit_byte(code, opcodes::I64_LE_S),
            WasmInstr::I64GeU => Self::emit_byte(code, opcodes::I64_GE_U),
            WasmInstr::I64GeS => Self::emit_byte(code, opcodes::I64_GE_S),

            // Arithmetic (i32)
            WasmInstr::I32Add => Self::emit_byte(code, opcodes::I32_ADD),
            WasmInstr::I32Sub => Self::emit_byte(code, opcodes::I32_SUB),
            WasmInstr::I32Mul => Self::emit_byte(code, opcodes::I32_MUL),
            WasmInstr::I32DivU => Self::emit_byte(code, opcodes::I32_DIV_U),
            WasmInstr::I32DivS => Self::emit_byte(code, opcodes::I32_DIV_S),
            WasmInstr::I32RemU => Self::emit_byte(code, opcodes::I32_REM_U),
            WasmInstr::I32And => Self::emit_byte(code, opcodes::I32_AND),
            WasmInstr::I32Or => Self::emit_byte(code, opcodes::I32_OR),
            WasmInstr::I32Xor => Self::emit_byte(code, opcodes::I32_XOR),
            WasmInstr::I32Shl => Self::emit_byte(code, opcodes::I32_SHL),
            WasmInstr::I32ShrU => Self::emit_byte(code, opcodes::I32_SHR_U),
            WasmInstr::I32ShrS => Self::emit_byte(code, opcodes::I32_SHR_S),

            // Comparisons (i32)
            WasmInstr::I32Eqz => Self::emit_byte(code, opcodes::I32_EQZ),
            WasmInstr::I32Eq => Self::emit_byte(code, opcodes::I32_EQ),
            WasmInstr::I32Ne => Self::emit_byte(code, opcodes::I32_NE),
            WasmInstr::I32LtU => Self::emit_byte(code, opcodes::I32_LT_U),
            WasmInstr::I32LtS => Self::emit_byte(code, opcodes::I32_LT_S),
            WasmInstr::I32GtU => Self::emit_byte(code, opcodes::I32_GT_U),
            WasmInstr::I32GtS => Self::emit_byte(code, opcodes::I32_GT_S),
            WasmInstr::I32LeU => Self::emit_byte(code, opcodes::I32_LE_U),
            WasmInstr::I32LeS => Self::emit_byte(code, opcodes::I32_LE_S),
            WasmInstr::I32GeU => Self::emit_byte(code, opcodes::I32_GE_U),
            WasmInstr::I32GeS => Self::emit_byte(code, opcodes::I32_GE_S),

            // Control flow
            WasmInstr::Unreachable => Self::emit_byte(code, opcodes::UNREACHABLE),
            WasmInstr::Block { result_type } => {
                Self::emit_byte(code, opcodes::BLOCK);
                if let Some(rt) = result_type {
                    Self::emit_byte(code, *rt);
                } else {
                    Self::emit_byte(code, opcodes::TYPE_VOID);
                }
            }
            WasmInstr::Loop { result_type } => {
                Self::emit_byte(code, opcodes::LOOP);
                if let Some(rt) = result_type {
                    Self::emit_byte(code, *rt);
                } else {
                    Self::emit_byte(code, opcodes::TYPE_VOID);
                }
            }
            WasmInstr::If { result_type } => {
                Self::emit_byte(code, opcodes::IF);
                if let Some(rt) = result_type {
                    Self::emit_byte(code, *rt);
                } else {
                    Self::emit_byte(code, opcodes::TYPE_VOID);
                }
            }
            WasmInstr::Else => Self::emit_byte(code, opcodes::ELSE),
            WasmInstr::End => Self::emit_byte(code, opcodes::END),
            WasmInstr::Br { label } => {
                Self::emit_byte(code, opcodes::BR);
                Self::emit_leb128_unsigned(*label as u64, code);
            }
            WasmInstr::BrIf { label } => {
                Self::emit_byte(code, opcodes::BR_IF);
                Self::emit_leb128_unsigned(*label as u64, code);
            }
            WasmInstr::Return => Self::emit_byte(code, opcodes::RETURN),
            WasmInstr::Call { func_idx } => {
                Self::emit_byte(code, opcodes::CALL);
                Self::emit_leb128_unsigned(*func_idx as u64, code);
            }
            WasmInstr::CallIndirect { type_idx } => {
                Self::emit_byte(code, opcodes::CALL_INDIRECT);
                Self::emit_leb128_unsigned(*type_idx as u64, code);
                Self::emit_byte(code, 0);  // table index
            }

            // Memory
            WasmInstr::LocalGet(idx) => {
                Self::emit_byte(code, opcodes::LOCAL_GET);
                Self::emit_leb128_unsigned(*idx as u64, code);
            }
            WasmInstr::LocalSet(idx) => {
                Self::emit_byte(code, opcodes::LOCAL_SET);
                Self::emit_leb128_unsigned(*idx as u64, code);
            }
            WasmInstr::LocalTee(idx) => {
                Self::emit_byte(code, opcodes::LOCAL_TEE);
                Self::emit_leb128_unsigned(*idx as u64, code);
            }
            WasmInstr::GlobalGet(idx) => {
                Self::emit_byte(code, opcodes::GLOBAL_GET);
                Self::emit_leb128_unsigned(*idx as u64, code);
            }
            WasmInstr::GlobalSet(idx) => {
                Self::emit_byte(code, opcodes::GLOBAL_SET);
                Self::emit_leb128_unsigned(*idx as u64, code);
            }
            WasmInstr::I64Load { align, offset } => {
                Self::emit_byte(code, opcodes::I64_LOAD);
                Self::emit_leb128_unsigned(*align as u64, code);
                Self::emit_leb128_unsigned(*offset as u64, code);
            }
            WasmInstr::I64Store { align, offset } => {
                Self::emit_byte(code, opcodes::I64_STORE);
                Self::emit_leb128_unsigned(*align as u64, code);
                Self::emit_leb128_unsigned(*offset as u64, code);
            }
            WasmInstr::I32Load { align, offset } => {
                Self::emit_byte(code, opcodes::I32_LOAD);
                Self::emit_leb128_unsigned(*align as u64, code);
                Self::emit_leb128_unsigned(*offset as u64, code);
            }
            WasmInstr::I32Store { align, offset } => {
                Self::emit_byte(code, opcodes::I32_STORE);
                Self::emit_leb128_unsigned(*align as u64, code);
                Self::emit_leb128_unsigned(*offset as u64, code);
            }
            WasmInstr::I32Load8U { align, offset } => {
                Self::emit_byte(code, opcodes::I32_LOAD8_U);
                Self::emit_leb128_unsigned(*align as u64, code);
                Self::emit_leb128_unsigned(*offset as u64, code);
            }
            WasmInstr::I32Store8 { align, offset } => {
                Self::emit_byte(code, opcodes::I32_STORE8);
                Self::emit_leb128_unsigned(*align as u64, code);
                Self::emit_leb128_unsigned(*offset as u64, code);
            }
            WasmInstr::MemorySize => {
                Self::emit_byte(code, opcodes::MEMORY_SIZE);
                Self::emit_byte(code, 0);
            }
            WasmInstr::MemoryGrow => {
                Self::emit_byte(code, opcodes::MEMORY_GROW);
                Self::emit_byte(code, 0);
            }

            // Conversions
            WasmInstr::I32WrapI64 => Self::emit_byte(code, opcodes::I32_WRAP_I64),
            WasmInstr::I64ExtendI32U => Self::emit_byte(code, opcodes::I64_EXTEND_I32_U),
            WasmInstr::I64ExtendI32S => Self::emit_byte(code, opcodes::I64_EXTEND_I32_S),

            // Stack
            WasmInstr::Drop => Self::emit_byte(code, opcodes::DROP),
            WasmInstr::Select => Self::emit_byte(code, opcodes::SELECT),
            WasmInstr::Nop => Self::emit_byte(code, opcodes::NOP),

            // Branch table (advanced)
            WasmInstr::BrTable { .. } => {
                // Not implemented in basic version
                Self::emit_byte(code, opcodes::UNREACHABLE);
            }
        }
    }

    /// Generate complete WASM module
    fn generate_wasm(&mut self, _program: &AnalyzedProgram) -> Result<Vec<u8>> {
        let mut wasm = Vec::new();

        // Magic number + version
        wasm.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);

        // Type section
        self.emit_type_section(&mut wasm)?;

        // Import section
        self.emit_import_section(&mut wasm)?;

        // Function section
        self.emit_function_section(&mut wasm)?;

        // Memory section
        self.emit_memory_section(&mut wasm)?;

        // Global section
        self.emit_global_section(&mut wasm)?;

        // Export section
        self.emit_export_section(&mut wasm)?;

        // Code section
        self.emit_code_section(&mut wasm)?;

        // Data section (optional)
        // self.emit_data_section(&mut wasm)?;

        info!(
            "Generated WASM module: {} bytes, {} type sigs, {} imports",
            wasm.len(),
            self.type_signatures.len(),
            self.imports.len()
        );

        Ok(wasm)
    }

    fn emit_type_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        let mut section = Vec::new();

        // Count
        Self::emit_leb128_unsigned(self.type_signatures.len() as u64, &mut section);

        // Emit each type
        for (params, results) in &self.type_signatures {
            section.push(opcodes::TYPE_FUNC);

            // Parameters
            Self::emit_leb128_unsigned(params.len() as u64, &mut section);
            section.extend_from_slice(params);

            // Results
            Self::emit_leb128_unsigned(results.len() as u64, &mut section);
            section.extend_from_slice(results);
        }

        // Emit section with header
        wasm.push(sections::TYPE);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }

    fn emit_import_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        let mut section = Vec::new();

        // Count
        Self::emit_leb128_unsigned(self.imports.len() as u64, &mut section);

        // Emit each import
        for (module, name, type_idx) in &self.imports {
            // Module name
            Self::emit_leb128_unsigned(module.len() as u64, &mut section);
            section.extend_from_slice(module.as_bytes());

            // Function name
            Self::emit_leb128_unsigned(name.len() as u64, &mut section);
            section.extend_from_slice(name.as_bytes());

            // Import kind: 0 = function
            section.push(0x00);

            // Type index
            Self::emit_leb128_unsigned(*type_idx as u64, &mut section);
        }

        // Emit section
        wasm.push(sections::IMPORT);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }

    fn emit_function_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        let mut section = Vec::new();

        // Count of user-defined functions
        Self::emit_leb128_unsigned(0u64, &mut section);  // No user functions for MVP

        // Emit section
        wasm.push(sections::FUNCTION);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }

    fn emit_memory_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        let mut section = Vec::new();

        // Count
        section.push(0x01);  // 1 memory

        // Min pages
        Self::emit_leb128_unsigned(self.memory_pages as u64, &mut section);
        // Max pages (optional, set to 256)
        Self::emit_leb128_unsigned(256u64, &mut section);

        // Emit section
        wasm.push(sections::MEMORY);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }

    fn emit_global_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        if self.globals.is_empty() {
            return Ok(());
        }

        let mut section = Vec::new();

        // Count
        Self::emit_leb128_unsigned(self.globals.len() as u64, &mut section);

        // Emit each global
        for (typ, mutable, init_expr) in &self.globals {
            section.push(*typ);
            section.push(if *mutable { 0x01 } else { 0x00 });
            section.extend_from_slice(init_expr);
            section.push(opcodes::END);
        }

        // Emit section
        wasm.push(sections::GLOBAL);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }

    fn emit_export_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        let mut section = Vec::new();

        // Standard exports for all modules
        let exports = vec![
            ("execute_intent", 0u32, 0u8),  // function 0
            ("finalize_intent", 1u32, 0u8), // function 1
        ];

        // Count
        Self::emit_leb128_unsigned(exports.len() as u64, &mut section);

        // Emit each export
        for (name, idx, kind) in &exports {
            Self::emit_leb128_unsigned(name.len() as u64, &mut section);
            section.extend_from_slice(name.as_bytes());
            section.push(*kind);
            Self::emit_leb128_unsigned(*idx as u64, &mut section);
        }

        // Emit section
        wasm.push(sections::EXPORT);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }

    fn emit_code_section(&self, wasm: &mut Vec<u8>) -> Result<()> {
        let mut section = Vec::new();

        // Count
        Self::emit_leb128_unsigned(0u64, &mut section);  // No function bodies for MVP

        // Emit section
        wasm.push(sections::CODE);
        Self::emit_leb128_unsigned(section.len() as u64, wasm);
        wasm.extend_from_slice(&section);

        Ok(())
    }
}

pub fn generate(program: &AnalyzedProgram) -> Result<Vec<u8>> {
    let mut codegen = Codegen::new();
    codegen.generate_wasm(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leb128_encoding() {
        let mut code = Vec::new();
        Codegen::emit_leb128_unsigned(624485, &mut code);
        assert_eq!(code, vec![0xe5, 0x8e, 0x26]);
    }

    #[test]
    fn test_signed_leb128() {
        let mut code = Vec::new();
        Codegen::emit_leb128_signed(-2, &mut code);
        assert_eq!(code, vec![0x7e]);
    }

    #[test]
    fn test_basic_wasm_generation() -> Result<()> {
        let program = AnalyzedProgram {
            modules: vec![],
        };
        let wasm = generate(&program)?;

        // Check magic number
        assert_eq!(&wasm[0..4], &[0x00, 0x61, 0x73, 0x6d]);
        // Check version
        assert_eq!(&wasm[4..8], &[0x01, 0x00, 0x00, 0x00]);

        Ok(())
    }

    #[test]
    fn test_wasm_structure() -> Result<()> {
        let program = AnalyzedProgram {
            modules: vec![],
        };
        let wasm = generate(&program)?;

        // Module should have:
        // - Magic (4 bytes)
        // - Version (4 bytes)
        // - Type section (ID + size + content)
        // - Import section (ID + size + content)
        // And more...
        
        assert!(wasm.len() > 12, "WASM module too small");
        Ok(())
    }
