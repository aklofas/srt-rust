//! UniversalLabel — 16-byte SMPTE/MISB key. Filled in Task 3.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniversalLabel(pub [u8; 16]);
