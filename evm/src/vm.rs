use std::collections::HashMap;

use crate::u256::U256;
use sha3::{Digest, Keccak256};

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("stack underflow")]
    StackUnderflow,
    #[error("stack overflow")]
    StackOverflow,
    #[error("out of gas")]
    OutOfGas,
    #[error("invalid opcode {0:#04x}")]
    InvalidOpcode(u8),
    #[error("invalid jump destination {0}")]
    InvalidJump(usize),
    #[error("reverted")]
    Reverted,
    #[error("memory limit exceeded")]
    MemoryLimit,
    #[error("state-modifying opcode in a static call")]
    StaticViolation,
    #[error("call depth limit exceeded")]
    CallDepth,
}

const MAX_STACK: usize = 1024;
const MAX_MEMORY: usize = 1 << 20;
const MAX_CALL_DEPTH: u32 = 64;
const MAX_CODE_SIZE: usize = 24_576;

#[derive(Clone, Debug, Default)]
pub struct Log {
    pub address: U256,
    pub topics: Vec<U256>,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
}

pub struct CallContext {
    pub caller: U256,
    pub address: U256,
    pub value: U256,
    pub calldata: Vec<u8>,
    pub gas_limit: u64,
}

pub trait Host {
    fn sload(&self, address: &U256, key: &U256) -> U256;
    fn sstore(&mut self, address: &U256, key: U256, value: U256);
    fn balance(&self, address: &U256) -> U256;
    fn block_number(&self) -> u64;
    fn block_timestamp(&self) -> u64;
    fn chain_id(&self) -> u64;

    fn code(&self, _address: &U256) -> Vec<u8> {
        Vec::new()
    }
    fn set_code(&mut self, _address: &U256, _code: Vec<u8>) {}
    fn account_nonce(&self, _address: &U256) -> u64 {
        0
    }
    fn bump_nonce(&mut self, _address: &U256) {}
    fn transfer(&mut self, _from: &U256, _to: &U256, _value: U256) -> bool {
        true
    }
    fn snapshot(&mut self) -> usize {
        0
    }
    fn revert(&mut self, _snapshot: usize) {}
    fn commit_snapshot(&mut self, _snapshot: usize) {}
}

#[derive(Debug)]
pub struct ExecOutcome {
    pub success: bool,
    pub return_data: Vec<u8>,
    pub gas_used: u64,
    pub logs: Vec<Log>,
}

pub struct Interpreter<'a, H: Host> {
    code: &'a [u8],
    stack: Vec<U256>,
    memory: Vec<u8>,
    pc: usize,
    gas: u64,
    gas_limit: u64,
    jumpdests: Vec<bool>,
    ctx: &'a CallContext,
    host: &'a mut H,
    logs: Vec<Log>,
    return_data: Vec<u8>,
    ret_buffer: Vec<u8>,
    depth: u32,
    is_static: bool,
}

fn analyze_jumpdests(code: &[u8]) -> Vec<bool> {
    let mut dests = vec![false; code.len()];
    let mut i = 0;
    while i < code.len() {
        let op = code[i];
        if op == 0x5b {
            dests[i] = true;
        }
        if (0x60..=0x7f).contains(&op) {
            i += (op - 0x60) as usize + 1;
        }
        i += 1;
    }
    dests
}

pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    out
}

fn addr20(v: &U256) -> [u8; 20] {
    let b = v.to_big_endian();
    let mut a = [0u8; 20];
    a.copy_from_slice(&b[12..]);
    a
}

fn addr_to_u256(addr: &[u8; 20]) -> U256 {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr);
    U256::from_big_endian(&buf)
}

pub fn create_address(sender: &U256, nonce: u64) -> U256 {
    let s = addr20(sender);
    let nonce_bytes = if nonce == 0 {
        vec![]
    } else {
        nonce
            .to_be_bytes()
            .iter()
            .copied()
            .skip_while(|&b| b == 0)
            .collect::<Vec<u8>>()
    };
    let rlp = crate::rlp::encode_list(&[
        crate::rlp::encode_str(&s),
        crate::rlp::encode_str(&nonce_bytes),
    ]);
    let h = keccak256(&rlp);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    addr_to_u256(&a)
}

pub fn create2_address(sender: &U256, salt: &U256, init_code: &[u8]) -> U256 {
    let mut buf = Vec::with_capacity(1 + 20 + 32 + 32);
    buf.push(0xff);
    buf.extend_from_slice(&addr20(sender));
    buf.extend_from_slice(&salt.to_big_endian());
    buf.extend_from_slice(&keccak256(init_code));
    let h = keccak256(&buf);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    addr_to_u256(&a)
}

pub fn execute_frame<H: Host>(
    code: &[u8],
    ctx: &CallContext,
    host: &mut H,
    depth: u32,
    is_static: bool,
) -> Result<ExecOutcome, VmError> {
    let mut interp = Interpreter::new(code, ctx, host);
    interp.depth = depth;
    interp.is_static = is_static;
    interp.run()
}

impl<'a, H: Host> Interpreter<'a, H> {
    pub fn new(code: &'a [u8], ctx: &'a CallContext, host: &'a mut H) -> Self {
        Interpreter {
            code,
            stack: Vec::with_capacity(64),
            memory: Vec::new(),
            pc: 0,
            gas: 0,
            gas_limit: ctx.gas_limit,
            jumpdests: analyze_jumpdests(code),
            ctx,
            host,
            logs: Vec::new(),
            return_data: Vec::new(),
            ret_buffer: Vec::new(),
            depth: 0,
            is_static: false,
        }
    }

    fn charge(&mut self, amount: u64) -> Result<(), VmError> {
        self.gas = self.gas.saturating_add(amount);
        if self.gas > self.gas_limit {
            return Err(VmError::OutOfGas);
        }
        Ok(())
    }

    fn push(&mut self, v: U256) -> Result<(), VmError> {
        if self.stack.len() >= MAX_STACK {
            return Err(VmError::StackOverflow);
        }
        self.stack.push(v);
        Ok(())
    }

    fn pop(&mut self) -> Result<U256, VmError> {
        self.stack.pop().ok_or(VmError::StackUnderflow)
    }

    fn ensure_mem(&mut self, offset: usize, len: usize) -> Result<(), VmError> {
        if len == 0 {
            return Ok(());
        }
        let end = offset.checked_add(len).ok_or(VmError::MemoryLimit)?;
        if end > MAX_MEMORY {
            return Err(VmError::MemoryLimit);
        }
        if end > self.memory.len() {
            let new_len = ((end + 31) / 32) * 32;
            let words_added = (new_len.saturating_sub(self.memory.len())) / 32;
            self.charge(words_added as u64 * 3)?;
            self.memory.resize(new_len, 0);
        }
        Ok(())
    }

    fn mem_store(&mut self, offset: usize, data: &[u8]) -> Result<(), VmError> {
        self.ensure_mem(offset, data.len())?;
        self.memory[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn mem_load(&mut self, offset: usize, len: usize) -> Result<Vec<u8>, VmError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        self.ensure_mem(offset, len)?;
        Ok(self.memory[offset..offset + len].to_vec())
    }

    pub fn run(mut self) -> Result<ExecOutcome, VmError> {
        let result = self.exec_loop();
        match result {
            Ok(()) => Ok(ExecOutcome {
                success: true,
                return_data: self.return_data,
                gas_used: self.gas,
                logs: self.logs,
            }),
            Err(VmError::Reverted) => Ok(ExecOutcome {
                success: false,
                return_data: self.return_data,
                gas_used: self.gas,
                logs: Vec::new(),
            }),
            Err(e) => Err(e),
        }
    }

    fn exec_loop(&mut self) -> Result<(), VmError> {
        loop {
            if self.pc >= self.code.len() {
                return Ok(());
            }
            let op = self.code[self.pc];
            self.charge(base_gas(op))?;
            match op {
                0x00 => return Ok(()),
                0x01 => self.bin(|a, b| a.wrapping_add(b))?,
                0x02 => self.bin(|a, b| a.wrapping_mul(b))?,
                0x03 => self.bin(|a, b| a.wrapping_sub(b))?,
                0x04 => self.bin(|a, b| a.div_mod(b).0)?,
                0x05 => self.sdiv()?,
                0x06 => self.bin(|a, b| a.div_mod(b).1)?,
                0x07 => self.smod()?,
                0x08 => self.addmod()?,
                0x09 => self.mulmod()?,
                0x0a => self.exp()?,
                0x0b => self.signextend()?,
                0x10 => self.bin(|a, b| bool_u(a.lt(&b)))?,
                0x11 => self.bin(|a, b| bool_u(b.lt(&a)))?,
                0x12 => self.slt()?,
                0x13 => self.sgt()?,
                0x14 => self.bin(|a, b| bool_u(a == b))?,
                0x15 => {
                    let a = self.pop()?;
                    self.push(bool_u(a.is_zero()))?;
                }
                0x16 => self.bin(|a, b| a.and(b))?,
                0x17 => self.bin(|a, b| a.or(b))?,
                0x18 => self.bin(|a, b| a.xor(b))?,
                0x19 => {
                    let a = self.pop()?;
                    self.push(a.not())?;
                }
                0x1a => self.byte_op()?,
                0x1b => self.bin(shift_l)?,
                0x1c => self.bin(shift_r)?,
                0x1d => self.sar()?,
                0x20 => self.keccak()?,
                0x30 => {
                    let a = self.ctx.address;
                    self.push(a)?;
                }
                0x31 => {
                    let a = self.pop()?;
                    let b = self.host.balance(&a);
                    self.push(b)?;
                }
                0x32 | 0x33 => self.push(self.ctx.caller)?,
                0x34 => self.push(self.ctx.value)?,
                0x35 => self.calldataload()?,
                0x36 => self.push(U256::from_u64(self.ctx.calldata.len() as u64))?,
                0x37 => self.calldatacopy()?,
                0x38 => self.push(U256::from_u64(self.code.len() as u64))?,
                0x39 => self.codecopy()?,
                0x3a => self.push(U256::ZERO)?,
                0x3b => self.extcodesize()?,
                0x3c => self.extcodecopy()?,
                0x3d => self.push(U256::from_u64(self.ret_buffer.len() as u64))?,
                0x3e => self.returndatacopy()?,
                0x3f => self.extcodehash()?,
                0x43 => self.push(U256::from_u64(self.host.block_number()))?,
                0x42 => self.push(U256::from_u64(self.host.block_timestamp()))?,
                0x46 => self.push(U256::from_u64(self.host.chain_id()))?,
                0x47 => {
                    let a = self.host.balance(&self.ctx.address);
                    self.push(a)?;
                }
                0x50 => {
                    self.pop()?;
                }
                0x51 => self.mload()?,
                0x52 => self.mstore()?,
                0x53 => self.mstore8()?,
                0x54 => {
                    let key = self.pop()?;
                    let v = self.host.sload(&self.ctx.address, &key);
                    self.push(v)?;
                }
                0x55 => {
                    if self.is_static {
                        return Err(VmError::StaticViolation);
                    }
                    self.charge(20_000)?;
                    let key = self.pop()?;
                    let val = self.pop()?;
                    let addr = self.ctx.address;
                    self.host.sstore(&addr, key, val);
                }
                0x56 => {
                    let dest = self.pop()?;
                    self.jump(dest)?;
                    continue;
                }
                0x57 => {
                    let dest = self.pop()?;
                    let cond = self.pop()?;
                    if !cond.is_zero() {
                        self.jump(dest)?;
                        continue;
                    }
                }
                0x58 => self.push(U256::from_u64(self.pc as u64))?,
                0x59 => self.push(U256::from_u64(self.memory.len() as u64))?,
                0x5a => self.push(U256::from_u64(self.gas_limit.saturating_sub(self.gas)))?,
                0x5b => {}
                0x60..=0x7f => {
                    let n = (op - 0x60) as usize + 1;
                    let start = self.pc + 1;
                    let end = (start + n).min(self.code.len());
                    let mut bytes = [0u8; 32];
                    let slice = &self.code[start..end];
                    bytes[32 - slice.len()..].copy_from_slice(slice);
                    self.push(U256::from_big_endian(&bytes[32 - n..]))?;
                    self.pc = start + n;
                    continue;
                }
                0x80..=0x8f => {
                    let n = (op - 0x80) as usize;
                    if n >= self.stack.len() {
                        return Err(VmError::StackUnderflow);
                    }
                    let v = self.stack[self.stack.len() - 1 - n];
                    self.push(v)?;
                }
                0x90..=0x9f => {
                    let n = (op - 0x90) as usize + 1;
                    let len = self.stack.len();
                    if n >= len {
                        return Err(VmError::StackUnderflow);
                    }
                    self.stack.swap(len - 1, len - 1 - n);
                }
                0xa0..=0xa4 => {
                    if self.is_static {
                        return Err(VmError::StaticViolation);
                    }
                    self.log_op(op - 0xa0)?
                }
                0xf0 => self.do_create(false)?,
                0xf5 => self.do_create(true)?,
                0xf1 => self.do_call(CallKind::Call)?,
                0xf2 => self.do_call(CallKind::CallCode)?,
                0xf4 => self.do_call(CallKind::DelegateCall)?,
                0xfa => self.do_call(CallKind::StaticCall)?,
                0xf3 => {
                    let (off, len) = (self.pop()?, self.pop()?);
                    self.return_data = self.mem_load(off.low_usize(), len.low_usize())?;
                    return Ok(());
                }
                0xfd => {
                    let (off, len) = (self.pop()?, self.pop()?);
                    self.return_data = self.mem_load(off.low_usize(), len.low_usize())?;
                    return Err(VmError::Reverted);
                }
                0xfe => return Err(VmError::InvalidOpcode(0xfe)),
                0xff => {
                    if self.is_static {
                        return Err(VmError::StaticViolation);
                    }
                    let beneficiary = self.pop()?;
                    let bal = self.host.balance(&self.ctx.address);
                    if !bal.is_zero() {
                        self.host.transfer(&self.ctx.address, &beneficiary, bal);
                    }
                    return Ok(());
                }
                _ => return Err(VmError::InvalidOpcode(op)),
            }
            self.pc += 1;
        }
    }

    fn do_call(&mut self, kind: CallKind) -> Result<(), VmError> {
        self.charge(100)?;
        let gas = self.pop()?;
        let target = self.pop()?;
        let value = match kind {
            CallKind::Call | CallKind::CallCode => self.pop()?,
            CallKind::DelegateCall | CallKind::StaticCall => U256::ZERO,
        };
        let args_off = self.pop()?.low_usize();
        let args_len = self.pop()?.low_usize();
        let ret_off = self.pop()?.low_usize();
        let ret_len = self.pop()?.low_usize();

        if self.is_static && !value.is_zero() {
            return Err(VmError::StaticViolation);
        }

        let calldata = self.mem_load(args_off, args_len)?;

        if self.depth >= MAX_CALL_DEPTH {
            self.ret_buffer.clear();
            return self.push(U256::ZERO);
        }

        let (storage_addr, code_addr, caller, call_value, sub_static) = match kind {
            CallKind::Call => (target, target, self.ctx.address, value, self.is_static),
            CallKind::StaticCall => (target, target, self.ctx.address, U256::ZERO, true),
            CallKind::DelegateCall => (
                self.ctx.address,
                target,
                self.ctx.caller,
                self.ctx.value,
                self.is_static,
            ),
            CallKind::CallCode => (
                self.ctx.address,
                target,
                self.ctx.address,
                value,
                self.is_static,
            ),
        };

        let snap = self.host.snapshot();
        if kind == CallKind::Call && !call_value.is_zero() {
            if self.host.balance(&self.ctx.address).lt(&call_value) {
                self.host.revert(snap);
                self.ret_buffer.clear();
                return self.push(U256::ZERO);
            }
            self.host.transfer(&self.ctx.address, &target, call_value);
        }

        let code = self.host.code(&code_addr);
        if code.is_empty() {
            self.host.commit_snapshot(snap);
            self.ret_buffer.clear();
            return self.push(U256::ONE);
        }

        let forwarded = self
            .gas_limit
            .saturating_sub(self.gas)
            .min(gas.low_u64_saturating());
        let subctx = CallContext {
            caller,
            address: storage_addr,
            value: call_value,
            calldata,
            gas_limit: forwarded.max(1),
        };
        let outcome = execute_frame(&code, &subctx, self.host, self.depth + 1, sub_static)?;
        self.charge(outcome.gas_used)?;
        if outcome.success {
            self.host.commit_snapshot(snap);
            self.logs.extend(outcome.logs);
        } else {
            self.host.revert(snap);
        }
        self.ret_buffer = outcome.return_data.clone();
        let n = ret_len.min(outcome.return_data.len());
        if n > 0 {
            self.mem_store(ret_off, &outcome.return_data[..n])?;
        }
        self.push(bool_u(outcome.success))
    }

    fn do_create(&mut self, is_create2: bool) -> Result<(), VmError> {
        if self.is_static {
            return Err(VmError::StaticViolation);
        }
        self.charge(32_000)?;
        let value = self.pop()?;
        let off = self.pop()?.low_usize();
        let len = self.pop()?.low_usize();
        let salt = if is_create2 { Some(self.pop()?) } else { None };
        let init = self.mem_load(off, len)?;

        if self.depth >= MAX_CALL_DEPTH {
            self.ret_buffer.clear();
            return self.push(U256::ZERO);
        }

        let sender = self.ctx.address;
        let new_addr = match &salt {
            Some(s) => create2_address(&sender, s, &init),
            None => {
                let nonce = self.host.account_nonce(&sender);
                create_address(&sender, nonce)
            }
        };
        self.host.bump_nonce(&sender);

        let snap = self.host.snapshot();
        if !value.is_zero() {
            if self.host.balance(&sender).lt(&value) {
                self.host.revert(snap);
                self.ret_buffer.clear();
                return self.push(U256::ZERO);
            }
            self.host.transfer(&sender, &new_addr, value);
        }

        let subctx = CallContext {
            caller: sender,
            address: new_addr,
            value,
            calldata: vec![],
            gas_limit: self.gas_limit.saturating_sub(self.gas).max(1),
        };
        let outcome = execute_frame(&init, &subctx, self.host, self.depth + 1, false)?;
        self.charge(outcome.gas_used)?;

        if outcome.success && outcome.return_data.len() <= MAX_CODE_SIZE {
            self.host.set_code(&new_addr, outcome.return_data.clone());
            self.host.commit_snapshot(snap);
            self.logs.extend(outcome.logs);
            self.ret_buffer.clear();
            self.push(new_addr)
        } else {
            self.host.revert(snap);
            self.ret_buffer = outcome.return_data;
            self.push(U256::ZERO)
        }
    }

    fn extcodesize(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let code = self.host.code(&a);
        self.push(U256::from_u64(code.len() as u64))
    }

    fn extcodecopy(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let dest = self.pop()?.low_usize();
        let off = self.pop()?.low_usize();
        let len = self.pop()?.low_usize();
        let code = self.host.code(&a);
        let mut data = vec![0u8; len];
        for (i, b) in data.iter_mut().enumerate() {
            if let Some(v) = code.get(off + i) {
                *b = *v;
            }
        }
        self.mem_store(dest, &data)
    }

    fn extcodehash(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let code = self.host.code(&a);
        if code.is_empty() {
            self.push(U256::ZERO)
        } else {
            self.push(U256::from_big_endian(&keccak256(&code)))
        }
    }

    fn returndatacopy(&mut self) -> Result<(), VmError> {
        let dest = self.pop()?.low_usize();
        let off = self.pop()?.low_usize();
        let len = self.pop()?.low_usize();
        let mut data = vec![0u8; len];
        for (i, b) in data.iter_mut().enumerate() {
            if let Some(v) = self.ret_buffer.get(off + i) {
                *b = *v;
            }
        }
        self.mem_store(dest, &data)
    }

    fn bin<F: Fn(U256, U256) -> U256>(&mut self, f: F) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        self.push(f(a, b))
    }

    fn jump(&mut self, dest: U256) -> Result<(), VmError> {
        let d = dest.low_usize();
        if !dest.bits_fit_usize() || d >= self.jumpdests.len() || !self.jumpdests[d] {
            return Err(VmError::InvalidJump(d));
        }
        self.pc = d;
        Ok(())
    }

    fn sdiv(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        if b.is_zero() {
            return self.push(U256::ZERO);
        }
        let (na, nb) = (a.is_neg(), b.is_neg());
        let ua = if na { a.neg() } else { a };
        let ub = if nb { b.neg() } else { b };
        let (q, _) = ua.div_mod(ub);
        self.push(if na ^ nb { q.neg() } else { q })
    }

    fn smod(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        if b.is_zero() {
            return self.push(U256::ZERO);
        }
        let na = a.is_neg();
        let ua = if na { a.neg() } else { a };
        let ub = if b.is_neg() { b.neg() } else { b };
        let (_, r) = ua.div_mod(ub);
        self.push(if na { r.neg() } else { r })
    }

    fn addmod(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        let m = self.pop()?;
        if m.is_zero() {
            return self.push(U256::ZERO);
        }
        let (sum, _) = a.overflowing_add(b);
        let (_, r) = sum.div_mod(m);
        self.push(r)
    }

    fn mulmod(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        let m = self.pop()?;
        if m.is_zero() {
            return self.push(U256::ZERO);
        }
        let (_, r) = a.wrapping_mul(b).div_mod(m);
        self.push(r)
    }

    fn exp(&mut self) -> Result<(), VmError> {
        let mut base = self.pop()?;
        let mut e = self.pop()?;
        let mut result = U256::ONE;
        while !e.is_zero() {
            if e.bit(0) {
                result = result.wrapping_mul(base);
            }
            base = base.wrapping_mul(base);
            e = e.shr(1);
        }
        self.push(result)
    }

    fn signextend(&mut self) -> Result<(), VmError> {
        let i = self.pop()?;
        let x = self.pop()?;
        if i.low_u64() >= 31 {
            return self.push(x);
        }
        let bit = (i.low_usize() * 8) + 7;
        let mask = U256::ONE.shl(bit + 1).wrapping_sub(U256::ONE);
        if x.bit(bit) {
            self.push(x.or(mask.not()))
        } else {
            self.push(x.and(mask))
        }
    }

    fn slt(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        self.push(bool_u(signed_lt(a, b)))
    }

    fn sgt(&mut self) -> Result<(), VmError> {
        let a = self.pop()?;
        let b = self.pop()?;
        self.push(bool_u(signed_lt(b, a)))
    }

    fn sar(&mut self) -> Result<(), VmError> {
        let sh = self.pop()?;
        let v = self.pop()?;
        let neg = v.is_neg();
        let s = sh.low_usize();
        if !sh.bits_fit_usize() || s >= 256 {
            return self.push(if neg { U256::ZERO.not() } else { U256::ZERO });
        }
        let mut r = v.shr(s);
        if neg {
            let mask = U256::ONE.shl(256 - s).wrapping_sub(U256::ONE).not();
            r = r.or(mask);
        }
        self.push(r)
    }

    fn byte_op(&mut self) -> Result<(), VmError> {
        let i = self.pop()?;
        let x = self.pop()?;
        if i.low_u64() >= 32 {
            return self.push(U256::ZERO);
        }
        let bytes = x.to_big_endian();
        self.push(U256::from_u64(bytes[i.low_usize()] as u64))
    }

    fn keccak(&mut self) -> Result<(), VmError> {
        let off = self.pop()?;
        let len = self.pop()?;
        let data = self.mem_load(off.low_usize(), len.low_usize())?;
        self.charge(30 + 6 * ((len.low_usize() as u64 + 31) / 32))?;
        self.push(U256::from_big_endian(&keccak256(&data)))
    }

    fn calldataload(&mut self) -> Result<(), VmError> {
        let off = self.pop()?.low_usize();
        let mut buf = [0u8; 32];
        for (i, b) in buf.iter_mut().enumerate() {
            if let Some(v) = self.ctx.calldata.get(off + i) {
                *b = *v;
            }
        }
        self.push(U256::from_big_endian(&buf))
    }

    fn calldatacopy(&mut self) -> Result<(), VmError> {
        let dest = self.pop()?.low_usize();
        let off = self.pop()?.low_usize();
        let len = self.pop()?.low_usize();
        let mut data = vec![0u8; len];
        for (i, b) in data.iter_mut().enumerate() {
            if let Some(v) = self.ctx.calldata.get(off + i) {
                *b = *v;
            }
        }
        self.mem_store(dest, &data)
    }

    fn codecopy(&mut self) -> Result<(), VmError> {
        let dest = self.pop()?.low_usize();
        let off = self.pop()?.low_usize();
        let len = self.pop()?.low_usize();
        let mut data = vec![0u8; len];
        for (i, b) in data.iter_mut().enumerate() {
            if let Some(v) = self.code.get(off + i) {
                *b = *v;
            }
        }
        self.mem_store(dest, &data)
    }

    fn mload(&mut self) -> Result<(), VmError> {
        let off = self.pop()?.low_usize();
        let data = self.mem_load(off, 32)?;
        self.push(U256::from_big_endian(&data))
    }

    fn mstore(&mut self) -> Result<(), VmError> {
        let off = self.pop()?.low_usize();
        let val = self.pop()?;
        self.mem_store(off, &val.to_big_endian())
    }

    fn mstore8(&mut self) -> Result<(), VmError> {
        let off = self.pop()?.low_usize();
        let val = self.pop()?;
        self.mem_store(off, &[val.to_big_endian()[31]])
    }

    fn log_op(&mut self, topic_count: u8) -> Result<(), VmError> {
        let off = self.pop()?.low_usize();
        let len = self.pop()?.low_usize();
        let mut topics = Vec::new();
        for _ in 0..topic_count {
            topics.push(self.pop()?);
        }
        let data = self.mem_load(off, len)?;
        self.charge(375 * topic_count as u64 + 8 * len as u64)?;
        self.logs.push(Log {
            address: self.ctx.address,
            topics,
            data,
        });
        Ok(())
    }
}

fn bool_u(b: bool) -> U256 {
    if b {
        U256::ONE
    } else {
        U256::ZERO
    }
}

fn signed_lt(a: U256, b: U256) -> bool {
    match (a.is_neg(), b.is_neg()) {
        (true, false) => true,
        (false, true) => false,
        _ => a.lt(&b),
    }
}

fn shift_l(sh: U256, v: U256) -> U256 {
    if !sh.bits_fit_usize() || sh.low_usize() >= 256 {
        U256::ZERO
    } else {
        v.shl(sh.low_usize())
    }
}

fn shift_r(sh: U256, v: U256) -> U256 {
    if !sh.bits_fit_usize() || sh.low_usize() >= 256 {
        U256::ZERO
    } else {
        v.shr(sh.low_usize())
    }
}

fn base_gas(op: u8) -> u64 {
    match op {
        0x00 | 0x5b => 1,
        0x54 => 200,
        0x60..=0x9f => 3,
        0x01 | 0x02 | 0x03 | 0x10..=0x1d => 3,
        0x20 => 30,
        0x51..=0x53 => 3,
        0x56 | 0x57 => 8,
        0x3b | 0x3c | 0x3f => 100,
        _ => 2,
    }
}

type Snapshot = (
    HashMap<(U256, U256), U256>,
    HashMap<U256, U256>,
    HashMap<U256, Vec<u8>>,
    HashMap<U256, u64>,
);

#[derive(Default)]
pub struct MemHost {
    pub storage: HashMap<(U256, U256), U256>,
    pub balances: HashMap<U256, U256>,
    pub codes: HashMap<U256, Vec<u8>>,
    pub nonces: HashMap<U256, u64>,
    pub block_number: u64,
    pub timestamp: u64,
    pub chain_id: u64,
    snapshots: Vec<Snapshot>,
}

impl Host for MemHost {
    fn sload(&self, address: &U256, key: &U256) -> U256 {
        self.storage
            .get(&(*address, *key))
            .copied()
            .unwrap_or(U256::ZERO)
    }
    fn sstore(&mut self, address: &U256, key: U256, value: U256) {
        self.storage.insert((*address, key), value);
    }
    fn balance(&self, address: &U256) -> U256 {
        self.balances.get(address).copied().unwrap_or(U256::ZERO)
    }
    fn block_number(&self) -> u64 {
        self.block_number
    }
    fn block_timestamp(&self) -> u64 {
        self.timestamp
    }
    fn chain_id(&self) -> u64 {
        self.chain_id
    }
    fn code(&self, address: &U256) -> Vec<u8> {
        self.codes.get(address).cloned().unwrap_or_default()
    }
    fn set_code(&mut self, address: &U256, code: Vec<u8>) {
        self.codes.insert(*address, code);
    }
    fn account_nonce(&self, address: &U256) -> u64 {
        self.nonces.get(address).copied().unwrap_or(0)
    }
    fn bump_nonce(&mut self, address: &U256) {
        *self.nonces.entry(*address).or_insert(0) += 1;
    }
    fn transfer(&mut self, from: &U256, to: &U256, value: U256) -> bool {
        let bal = self.balances.get(from).copied().unwrap_or(U256::ZERO);
        if bal.lt(&value) {
            return false;
        }
        self.balances.insert(*from, bal.wrapping_sub(value));
        let tb = self.balances.get(to).copied().unwrap_or(U256::ZERO);
        self.balances.insert(*to, tb.wrapping_add(value));
        true
    }
    fn snapshot(&mut self) -> usize {
        self.snapshots.push((
            self.storage.clone(),
            self.balances.clone(),
            self.codes.clone(),
            self.nonces.clone(),
        ));
        self.snapshots.len()
    }
    fn revert(&mut self, snapshot: usize) {
        while self.snapshots.len() >= snapshot && !self.snapshots.is_empty() {
            let (s, b, c, n) = self.snapshots.pop().unwrap();
            if self.snapshots.len() + 1 == snapshot {
                self.storage = s;
                self.balances = b;
                self.codes = c;
                self.nonces = n;
                return;
            }
        }
    }
    fn commit_snapshot(&mut self, snapshot: usize) {
        if self.snapshots.len() >= snapshot && !self.snapshots.is_empty() {
            self.snapshots.truncate(snapshot - 1);
        }
    }
}

impl std::hash::Hash for U256 {
    fn hash<S: std::hash::Hasher>(&self, state: &mut S) {
        self.0.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_code(code: &[u8], calldata: Vec<u8>) -> ExecOutcome {
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::from_u64(0xcafe),
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata,
            gas_limit: 10_000_000,
        };
        Interpreter::new(code, &ctx, &mut host).run().unwrap()
    }

    #[test]
    fn add_and_return() {
        let code = hex::decode("600560070160005260206000f3").unwrap();
        let out = run_code(&code, vec![]);
        assert!(out.success);
        assert_eq!(U256::from_big_endian(&out.return_data), U256::from_u64(12));
    }

    #[test]
    fn mstore_mload_return() {
        let code = hex::decode("602a60005260206000f3").unwrap();
        let out = run_code(&code, vec![]);
        assert_eq!(U256::from_big_endian(&out.return_data), U256::from_u64(42));
    }

    #[test]
    fn jumpi_skips() {
        let code = hex::decode("6001600657005b60ff60005260206000f3").unwrap();
        let out = run_code(&code, vec![]);
        assert!(out.success);
        assert_eq!(
            U256::from_big_endian(&out.return_data),
            U256::from_u64(0xff)
        );
    }

    #[test]
    fn keccak_of_empty_memory_region() {
        let code = hex::decode("60006000208060005260206000f3").unwrap();
        let out = run_code(&code, vec![]);
        assert!(out.success);
    }

    #[test]
    fn revert_marks_failure() {
        let code = hex::decode("6000600afd").unwrap();
        let out = run_code(&code, vec![]);
        assert!(!out.success);
    }

    #[test]
    fn sstore_sload_via_host() {
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 10_000_000,
        };
        let code = hex::decode("602a60005560005460005260206000f3").unwrap();
        let out = Interpreter::new(&code, &ctx, &mut host).run().unwrap();
        assert_eq!(U256::from_big_endian(&out.return_data), U256::from_u64(42));
        assert_eq!(
            host.sload(&U256::from_u64(0x1234), &U256::ZERO),
            U256::from_u64(42)
        );
    }

    #[test]
    fn solidity_style_storage_contract_dispatch() {
        let runtime = hex::decode(
            "60003560e01c80636057361d14601b5780632e64cec114602357005b600435600055005b60005460005260206000f3",
        )
        .unwrap();

        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let addr = U256::from_u64(0x1234);

        let mut store_call = hex::decode("6057361d").unwrap();
        store_call.extend_from_slice(&U256::from_u64(424242).to_big_endian());
        let ctx = CallContext {
            caller: U256::from_u64(0xcafe),
            address: addr,
            value: U256::ZERO,
            calldata: store_call,
            gas_limit: 10_000_000,
        };
        let out = Interpreter::new(&runtime, &ctx, &mut host).run().unwrap();
        assert!(out.success, "store(uint256) must succeed");

        let ctx2 = CallContext {
            caller: U256::from_u64(0xcafe),
            address: addr,
            value: U256::ZERO,
            calldata: hex::decode("2e64cec1").unwrap(),
            gas_limit: 10_000_000,
        };
        let out2 = Interpreter::new(&runtime, &ctx2, &mut host).run().unwrap();
        assert!(out2.success, "retrieve() must succeed");
        assert_eq!(
            U256::from_big_endian(&out2.return_data),
            U256::from_u64(424242),
            "retrieve() returns the value stored by the previous call (real SLOAD/SSTORE + ABI selector dispatch)"
        );
    }

    #[test]
    fn infinite_loop_halts_on_out_of_gas() {
        let code = hex::decode("5b600056").unwrap();
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 1_000_000,
        };
        let result = Interpreter::new(&code, &ctx, &mut host).run();
        assert!(
            matches!(result, Err(VmError::OutOfGas)),
            "an unconditional JUMP loop must terminate with OutOfGas, not hang: {result:?}"
        );
    }

    #[test]
    fn memory_bomb_is_bounded() {
        let code = hex::decode("600163ffffffff52").unwrap();
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 100_000_000,
        };
        let result = Interpreter::new(&code, &ctx, &mut host).run();
        assert!(
            matches!(result, Err(VmError::MemoryLimit) | Err(VmError::OutOfGas)),
            "a huge MSTORE offset must hit the memory cap, not allocate unbounded: {result:?}"
        );
    }

    #[test]
    fn jump_to_non_jumpdest_is_rejected() {
        let code = hex::decode("600356005b").unwrap();
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 1_000_000,
        };
        let result = Interpreter::new(&code, &ctx, &mut host).run();
        assert!(
            matches!(result, Err(VmError::InvalidJump(_))),
            "a JUMP into the middle of a PUSH must be rejected: {result:?}"
        );
    }

    #[test]
    fn stack_underflow_does_not_panic() {
        let code = hex::decode("01").unwrap();
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 1_000_000,
        };
        let result = Interpreter::new(&code, &ctx, &mut host).run();
        assert!(matches!(result, Err(VmError::StackUnderflow)));
    }

    #[test]
    fn static_call_blocks_sstore() {
        let code = hex::decode("602a600055").unwrap();
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 1_000_000,
        };
        let result = execute_frame(&code, &ctx, &mut host, 0, true);
        assert!(
            matches!(result, Err(VmError::StaticViolation)),
            "SSTORE inside a static call must be rejected: {result:?}"
        );
    }

    #[test]
    fn contract_calls_another_contract_and_reads_return() {
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };

        let callee = hex::decode("604560005260206000f3").unwrap();
        let callee_addr = U256::from_u64(0xbeef);
        host.set_code(&callee_addr, callee);

        let mut caller_code = Vec::new();
        caller_code.extend_from_slice(&[0x60, 0x20]);
        caller_code.extend_from_slice(&[0x60, 0x00]);
        caller_code.extend_from_slice(&[0x60, 0x00]);
        caller_code.extend_from_slice(&[0x60, 0x00]);
        caller_code.extend_from_slice(&[0x60, 0x00]);
        caller_code.extend_from_slice(&[0x61, 0xbe, 0xef]);
        caller_code.extend_from_slice(&[0x62, 0x0f, 0x42, 0x40]);
        caller_code.push(0xf1);
        caller_code.push(0x50);
        caller_code.extend_from_slice(&[0x60, 0x20, 0x60, 0x00, 0xf3]);

        let ctx = CallContext {
            caller: U256::from_u64(0xcafe),
            address: U256::from_u64(0x1234),
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 10_000_000,
        };
        let out = Interpreter::new(&caller_code, &ctx, &mut host)
            .run()
            .unwrap();
        assert!(out.success, "outer execution must succeed");
        assert_eq!(
            U256::from_big_endian(&out.return_data),
            U256::from_u64(0x45),
            "the caller must return the value (0x45) the callee wrote into its return buffer via CALL"
        );
    }

    #[test]
    fn delegatecall_runs_callee_code_in_caller_storage() {
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };

        let lib = hex::decode("602a600055").unwrap();
        let lib_addr = U256::from_u64(0xabcd);
        host.set_code(&lib_addr, lib);

        let mut code = Vec::new();
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x61, 0xab, 0xcd]);
        code.extend_from_slice(&[0x62, 0x0f, 0x42, 0x40]);
        code.push(0xf4);
        code.push(0x00);

        let self_addr = U256::from_u64(0x1234);
        let ctx = CallContext {
            caller: U256::from_u64(0xcafe),
            address: self_addr,
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 10_000_000,
        };
        let out = Interpreter::new(&code, &ctx, &mut host).run().unwrap();
        assert!(out.success);
        assert_eq!(
            host.sload(&self_addr, &U256::ZERO),
            U256::from_u64(42),
            "DELEGATECALL must run the library's SSTORE against the CALLER's storage, not the library's"
        );
        assert_eq!(
            host.sload(&lib_addr, &U256::ZERO),
            U256::ZERO,
            "the library's own storage must stay untouched under DELEGATECALL"
        );
    }

    #[test]
    fn create2_deploys_at_deterministic_address() {
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };

        let runtime = hex::decode("602a60005260206000f3").unwrap();
        let rt_len = runtime.len();
        let mut init = Vec::new();
        init.extend_from_slice(&[0x60, rt_len as u8]);
        init.extend_from_slice(&[0x60, 0x0c]);
        init.extend_from_slice(&[0x60, 0x00]);
        init.push(0x39);
        init.extend_from_slice(&[0x60, rt_len as u8]);
        init.extend_from_slice(&[0x60, 0x00]);
        init.push(0xf3);
        assert_eq!(
            init.len(),
            12,
            "init prologue must be 12 bytes before runtime"
        );
        init.extend_from_slice(&runtime);

        let sender = U256::from_u64(0x1234);
        let salt = U256::from_u64(0x9999);
        let expected = create2_address(&sender, &salt, &init);

        let ctx = CallContext {
            caller: U256::from_u64(0xcafe),
            address: sender,
            value: U256::ZERO,
            calldata: init.clone(),
            gas_limit: 10_000_000,
        };
        let mut create_code = Vec::new();
        create_code.extend_from_slice(&[0x60, init.len() as u8]);
        create_code.extend_from_slice(&[0x60, 0x00]);
        create_code.extend_from_slice(&[0x60, 0x00]);
        create_code.push(0x37);
        create_code.extend_from_slice(&[0x61, 0x99, 0x99]);
        create_code.extend_from_slice(&[0x60, init.len() as u8]);
        create_code.extend_from_slice(&[0x60, 0x00]);
        create_code.extend_from_slice(&[0x60, 0x00]);
        create_code.push(0xf5);
        create_code.extend_from_slice(&[0x60, 0x00, 0x52]);
        create_code.extend_from_slice(&[0x60, 0x20, 0x60, 0x00, 0xf3]);

        let out = Interpreter::new(&create_code, &ctx, &mut host)
            .run()
            .unwrap();
        assert!(out.success, "CREATE2 frame must succeed");
        let returned = U256::from_big_endian(&out.return_data);
        assert_eq!(
            returned, expected,
            "CREATE2 must deploy at keccak256(0xff++sender++salt++keccak(init))[12..]"
        );
        assert_eq!(
            host.code(&expected),
            runtime,
            "the deployed runtime code must match what the init code returned"
        );
    }

    #[test]
    fn call_depth_is_bounded() {
        let mut host = MemHost {
            chain_id: 1,
            ..Default::default()
        };
        let self_addr = U256::from_u64(0x1234);
        let mut code = Vec::new();
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x60, 0x00]);
        code.extend_from_slice(&[0x61, 0x12, 0x34]);
        code.extend_from_slice(&[0x62, 0x0f, 0x42, 0x40]);
        code.push(0xf1);
        code.push(0x00);
        host.set_code(&self_addr, code.clone());

        let ctx = CallContext {
            caller: U256::from_u64(0xcafe),
            address: self_addr,
            value: U256::ZERO,
            calldata: vec![],
            gas_limit: 500_000_000,
        };
        let result = execute_frame(&code, &ctx, &mut host, 0, false);
        assert!(
            result.is_ok(),
            "recursive self-calls must terminate via depth/gas bound, not overflow the native stack: {result:?}"
        );
    }
}
