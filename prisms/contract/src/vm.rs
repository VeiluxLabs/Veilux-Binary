use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("stack underflow")]
    StackUnderflow,
    #[error("stack overflow")]
    StackOverflow,
    #[error("out of gas: needed {needed}, had {had}")]
    OutOfGas { needed: u64, had: u64 },
    #[error("invalid opcode: {0:#04x}")]
    InvalidOpcode(u8),
    #[error("invalid jump destination: {0}")]
    InvalidJump(usize),
    #[error("division by zero")]
    DivByZero,
    #[error("execution reverted: {0}")]
    Reverted(String),
    #[error("program counter out of bounds")]
    PcOutOfBounds,
    #[error("truncated immediate operand")]
    TruncatedOperand,
}

pub const STOP: u8 = 0x00;
pub const ADD: u8 = 0x01;
pub const SUB: u8 = 0x02;
pub const MUL: u8 = 0x03;
pub const DIV: u8 = 0x04;
pub const MOD: u8 = 0x05;
pub const LT: u8 = 0x10;
pub const GT: u8 = 0x11;
pub const EQ: u8 = 0x12;
pub const ISZERO: u8 = 0x13;
pub const AND: u8 = 0x16;
pub const OR: u8 = 0x17;
pub const NOT: u8 = 0x19;
pub const POP: u8 = 0x50;
pub const DUP: u8 = 0x51;
pub const SWAP: u8 = 0x52;
pub const PUSH8: u8 = 0x60;
pub const SLOAD: u8 = 0x54;
pub const SSTORE: u8 = 0x55;
pub const JUMP: u8 = 0x56;
pub const JUMPI: u8 = 0x57;
pub const JUMPDEST: u8 = 0x5b;
pub const CALLER: u8 = 0x33;
pub const CALLVALUE: u8 = 0x34;
pub const ARG: u8 = 0x35;
pub const LOG: u8 = 0xa0;
pub const RETURN: u8 = 0xf3;
pub const REVERT: u8 = 0xfd;

const MAX_STACK: usize = 1024;

pub struct ExecContext {
    pub caller_hash: u64,
    pub call_value: u64,
    pub args: Vec<u64>,
}

pub struct ExecResult {
    pub return_value: Option<u64>,
    pub gas_used: u64,
    pub logs: Vec<u64>,
    pub reverted: bool,
}

pub struct Vm<'a> {
    code: &'a [u8],
    stack: Vec<u64>,
    pc: usize,
    gas: u64,
    gas_limit: u64,
    storage: &'a mut HashMap<u64, u64>,
    ctx: &'a ExecContext,
    logs: Vec<u64>,
    jumpdests: Vec<usize>,
}

impl<'a> Vm<'a> {
    pub fn new(
        code: &'a [u8],
        gas_limit: u64,
        storage: &'a mut HashMap<u64, u64>,
        ctx: &'a ExecContext,
    ) -> Self {
        let jumpdests = code
            .iter()
            .enumerate()
            .filter(|(_, &b)| b == JUMPDEST)
            .map(|(i, _)| i)
            .collect();
        Vm {
            code,
            stack: Vec::with_capacity(64),
            pc: 0,
            gas: 0,
            gas_limit,
            storage,
            ctx,
            logs: Vec::new(),
            jumpdests,
        }
    }

    fn charge(&mut self, amount: u64) -> Result<(), VmError> {
        self.gas += amount;
        if self.gas > self.gas_limit {
            return Err(VmError::OutOfGas {
                needed: self.gas,
                had: self.gas_limit,
            });
        }
        Ok(())
    }

    fn push(&mut self, v: u64) -> Result<(), VmError> {
        if self.stack.len() >= MAX_STACK {
            return Err(VmError::StackOverflow);
        }
        self.stack.push(v);
        Ok(())
    }

    fn pop(&mut self) -> Result<u64, VmError> {
        self.stack.pop().ok_or(VmError::StackUnderflow)
    }

    pub fn run(&mut self) -> Result<ExecResult, VmError> {
        loop {
            if self.pc >= self.code.len() {
                return Ok(self.finish(None, false));
            }
            let op = self.code[self.pc];
            self.charge(opcode_gas(op))?;

            match op {
                STOP => return Ok(self.finish(None, false)),
                ADD => {
                    let (a, b) = self.pop2()?;
                    self.push(a.wrapping_add(b))?;
                }
                SUB => {
                    let (a, b) = self.pop2()?;
                    self.push(a.wrapping_sub(b))?;
                }
                MUL => {
                    let (a, b) = self.pop2()?;
                    self.push(a.wrapping_mul(b))?;
                }
                DIV => {
                    let (a, b) = self.pop2()?;
                    if b == 0 {
                        return Err(VmError::DivByZero);
                    }
                    self.push(a / b)?;
                }
                MOD => {
                    let (a, b) = self.pop2()?;
                    if b == 0 {
                        return Err(VmError::DivByZero);
                    }
                    self.push(a % b)?;
                }
                LT => {
                    let (a, b) = self.pop2()?;
                    self.push((a < b) as u64)?;
                }
                GT => {
                    let (a, b) = self.pop2()?;
                    self.push((a > b) as u64)?;
                }
                EQ => {
                    let (a, b) = self.pop2()?;
                    self.push((a == b) as u64)?;
                }
                ISZERO => {
                    let a = self.pop()?;
                    self.push((a == 0) as u64)?;
                }
                AND => {
                    let (a, b) = self.pop2()?;
                    self.push(a & b)?;
                }
                OR => {
                    let (a, b) = self.pop2()?;
                    self.push(a | b)?;
                }
                NOT => {
                    let a = self.pop()?;
                    self.push(!a)?;
                }
                POP => {
                    self.pop()?;
                }
                DUP => {
                    let top = *self.stack.last().ok_or(VmError::StackUnderflow)?;
                    self.push(top)?;
                }
                SWAP => {
                    let n = self.stack.len();
                    if n < 2 {
                        return Err(VmError::StackUnderflow);
                    }
                    self.stack.swap(n - 1, n - 2);
                }
                PUSH8 => {
                    let start = self.pc + 1;
                    let end = start + 8;
                    if end > self.code.len() {
                        return Err(VmError::TruncatedOperand);
                    }
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&self.code[start..end]);
                    self.push(u64::from_be_bytes(buf))?;
                    self.pc = end;
                    continue;
                }
                SLOAD => {
                    let key = self.pop()?;
                    let v = self.storage.get(&key).copied().unwrap_or(0);
                    self.push(v)?;
                }
                SSTORE => {
                    let key = self.pop()?;
                    let val = self.pop()?;
                    self.storage.insert(key, val);
                }
                JUMP => {
                    let dest = self.pop()? as usize;
                    self.jump_to(dest)?;
                    continue;
                }
                JUMPI => {
                    let dest = self.pop()? as usize;
                    let cond = self.pop()?;
                    if cond != 0 {
                        self.jump_to(dest)?;
                        continue;
                    }
                }
                JUMPDEST => {}
                CALLER => {
                    let c = self.ctx.caller_hash;
                    self.push(c)?;
                }
                CALLVALUE => {
                    let v = self.ctx.call_value;
                    self.push(v)?;
                }
                ARG => {
                    let idx = self.pop()? as usize;
                    let v = self.ctx.args.get(idx).copied().unwrap_or(0);
                    self.push(v)?;
                }
                LOG => {
                    let v = self.pop()?;
                    self.logs.push(v);
                }
                RETURN => {
                    let v = self.pop()?;
                    return Ok(self.finish(Some(v), false));
                }
                REVERT => {
                    return Err(VmError::Reverted("REVERT opcode".into()));
                }
                other => return Err(VmError::InvalidOpcode(other)),
            }
            self.pc += 1;
        }
    }

    fn pop2(&mut self) -> Result<(u64, u64), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        Ok((a, b))
    }

    fn jump_to(&mut self, dest: usize) -> Result<(), VmError> {
        if !self.jumpdests.contains(&dest) {
            return Err(VmError::InvalidJump(dest));
        }
        self.pc = dest;
        Ok(())
    }

    fn finish(&self, return_value: Option<u64>, reverted: bool) -> ExecResult {
        ExecResult {
            return_value,
            gas_used: self.gas,
            logs: self.logs.clone(),
            reverted,
        }
    }
}

fn opcode_gas(op: u8) -> u64 {
    match op {
        STOP | JUMPDEST => 1,
        SLOAD => 200,
        SSTORE => 5_000,
        PUSH8 | DUP | SWAP | POP => 3,
        JUMP | JUMPI => 8,
        LOG => 375,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(code: &[u8], args: Vec<u64>) -> Result<ExecResult, VmError> {
        let mut storage = HashMap::new();
        let ctx = ExecContext {
            caller_hash: 42,
            call_value: 0,
            args,
        };
        let mut vm = Vm::new(code, 1_000_000, &mut storage, &ctx);
        vm.run()
    }

    #[test]
    fn push_add_return() {
        let mut code = vec![PUSH8];
        code.extend_from_slice(&5u64.to_be_bytes());
        code.push(PUSH8);
        code.extend_from_slice(&7u64.to_be_bytes());
        code.push(ADD);
        code.push(RETURN);
        let res = run(&code, vec![]).unwrap();
        assert_eq!(res.return_value, Some(12));
    }

    #[test]
    fn div_by_zero_errors() {
        let mut code = vec![PUSH8];
        code.extend_from_slice(&5u64.to_be_bytes());
        code.push(PUSH8);
        code.extend_from_slice(&0u64.to_be_bytes());
        code.push(DIV);
        assert!(matches!(run(&code, vec![]), Err(VmError::DivByZero)));
    }

    #[test]
    fn sstore_sload_roundtrip() {
        let mut code = vec![PUSH8];
        code.extend_from_slice(&99u64.to_be_bytes());
        code.push(PUSH8);
        code.extend_from_slice(&1u64.to_be_bytes());
        code.push(SSTORE);
        code.push(PUSH8);
        code.extend_from_slice(&1u64.to_be_bytes());
        code.push(SLOAD);
        code.push(RETURN);
        let res = run(&code, vec![]).unwrap();
        assert_eq!(res.return_value, Some(99));
    }

    #[test]
    fn invalid_jump_errors() {
        let code = vec![PUSH8, 0, 0, 0, 0, 0, 0, 0, 200, JUMP];
        assert!(matches!(run(&code, vec![]), Err(VmError::InvalidJump(_))));
    }
}
